//! 轮盘管理与执行协调；浮窗只提交选择，不在输入 Hook 内执行动作。
use crate::{
    radial_wheel::RingGeometry,
    scenes::CompiledScenes,
    wheel_scenes::{scene_scope, WheelSlotKey},
    wheels::{WheelDefinition, WheelPosition, WheelSlot},
    window_action::{WindowAction, WindowOperation},
    ActionSpec, Activation, BackendCommand, Bounds, Context, HitSource, Page, RadialWheel,
    RawWindowHandle, ShortcutAction, WheelAccent, WheelActionRef, WheelRequest, Wheels,
    WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowKind, WindowOptions, Workspace,
    WHEEL_SIZE,
};
use gpui::{prelude::*, px, size};
use omnitrigger_core::{DragEvent, DragPhase};
use raw_window_handle::HasWindowHandle;
use std::{collections::HashMap, time::Duration};

/// 先找名称和动作都保留的槽位，再处理改名。每轮先对齐原方位，再对应唯一的换位。
fn remap_wheel_layout(
    old: &[WheelSlot],
    new: &[WheelSlot],
    wheel: &WheelDefinition,
    process: Option<&str>,
    previous: &HashMap<WheelSlotKey, u64>,
    current: &mut HashMap<WheelSlotKey, u64>,
) {
    if old == new {
        for index in 0..old.len() {
            let key = WheelSlotKey::new(wheel, process, index);
            if let Some(id) = previous.get(&key) {
                current.insert(key, *id);
            }
        }
        return;
    }
    let cell = |slot: &WheelSlot, index: usize| {
        slot.position
            .map_or((0, index), |p| (p.ring, usize::from(p.sector)))
    };
    let mut mapping = vec![None; old.len()];
    let mut used = vec![false; new.len()];
    for check_label in [true, false] {
        let matches = |a: &WheelSlot, b: &WheelSlot| {
            a.scene == b.scene && a.action == b.action && (!check_label || a.label == b.label)
        };
        for (index, slot) in old.iter().enumerate() {
            if mapping[index].is_some() {
                continue;
            }
            let next = new.iter().enumerate().position(|(i, other)| {
                !used[i] && cell(other, i) == cell(slot, index) && matches(slot, other)
            });
            if let Some(next) = next {
                mapping[index] = Some(next);
                used[next] = true;
            }
        }
        for (index, slot) in old.iter().enumerate() {
            if mapping[index].is_some() {
                continue;
            }
            let mut candidates = new
                .iter()
                .enumerate()
                .filter(|(i, other)| !used[*i] && matches(slot, other));
            if let Some((next, _)) = candidates.next() {
                let ambiguous = candidates.next().is_some()
                    || old.iter().enumerate().any(|(i, other)| {
                        i != index && mapping[i].is_none() && matches(slot, other)
                    });
                if !ambiguous {
                    mapping[index] = Some(next);
                    used[next] = true;
                }
            }
        }
    }
    for (index, next) in mapping.into_iter().enumerate() {
        if let (Some(next), Some(id)) = (
            next,
            previous.get(&WheelSlotKey::new(wheel, process, index)),
        ) {
            current.insert(WheelSlotKey::new(wheel, process, next), *id);
        }
    }
}

fn remap_wheel_runtime_ids(old: &Wheels, new: &Wheels, ids: &mut HashMap<WheelSlotKey, u64>) {
    if ids.is_empty() || old == new {
        return;
    }
    let previous = std::mem::take(ids);
    for wheel in &new.wheels {
        let Some(before) = old.wheels.iter().find(|item| item.id == wheel.id) else {
            continue;
        };
        remap_wheel_layout(&before.slots, &wheel.slots, wheel, None, &previous, ids);
        for app in &wheel.app_overrides {
            if let Some(old_app) = before.override_for_process(Some(&app.process)) {
                remap_wheel_layout(
                    &old_app.slots,
                    &app.slots,
                    wheel,
                    Some(&app.process),
                    &previous,
                    ids,
                );
            }
        }
    }
}

#[derive(Clone)]
pub(super) struct ActiveWheel {
    definition: WheelDefinition,
    runtime_ids: Vec<u64>,
}

impl ActiveWheel {
    fn for_target(
        wheel: &WheelDefinition,
        scenes: &crate::scenes::CompiledScenes,
        process: Option<&str>,
        context: crate::wheel_scenes::TargetContext<'_>,
        ids: &mut std::collections::HashMap<crate::wheel_scenes::WheelSlotKey, u64>,
        triggers: &mut crate::service::TriggerService,
    ) -> Result<Self, crate::service::TriggerError> {
        use std::collections::hash_map::Entry;
        let (definition, keys) =
            crate::wheel_scenes::resolve_for_target(wheel, scenes, process, context);
        let mut runtime_ids = Vec::with_capacity(keys.len());
        for key in keys {
            let id = match ids.entry(key) {
                Entry::Occupied(slot) => *slot.get(),
                Entry::Vacant(slot) => *slot.insert(triggers.allocate_id()?),
            };
            runtime_ids.push(id);
        }
        Ok(Self {
            definition,
            runtime_ids,
        })
    }

    fn selection(&self, index: usize) -> Option<(u64, String, WheelSlot)> {
        Some((
            *self.runtime_ids.get(index)?,
            self.definition.name.clone(),
            self.definition.slots.get(index)?.clone(),
        ))
    }
}

pub(super) struct DragWheelSession {
    session: u64,
    binding_id: u64,
    wheel: ActiveWheel,
    origin: (i32, i32),
    cancel_radius_px: u32,
    scale: f32,
    target: omnitrigger_windows::ShortcutTarget,
}

#[derive(Clone, Copy)]
struct DragPlacement {
    origin: (i32, i32),
    position: (i32, i32),
    cancel_radius_px: u32,
    scale: f32,
}

pub(super) const RING_NAMES: [&str; 3] = ["内圈", "中圈", "外圈"];

/// 顶部显示与“更改”使用同一条绑定。全都停用时仍可打开原绑定修改。
pub(super) fn wheel_binding<'a>(
    bindings: &'a [crate::settings::Binding],
    wheel_id: &str,
) -> Option<&'a crate::settings::Binding> {
    bindings.iter().filter(|binding| {
        matches!(&binding.action, ActionSpec::ShowWheel(reference) if reference.wheel_id == wheel_id)
    }).min_by_key(|binding| !binding.enabled)
}

#[derive(Default)]
pub(super) struct WheelPreview {
    pub(super) selected: Option<(u8, u8)>,
}

impl WheelPreview {
    pub(super) fn selected_slot<'a>(&self, view: &'a WheelDefinition) -> Option<&'a WheelSlot> {
        let (ring, sector) = self.selected?;
        slot_at(view, ring, sector).map(|index| &view.slots[index])
    }
}

#[derive(Debug, PartialEq, Eq)]
enum PreviewAction {
    Describe,
    Notification,
    BrowseHistory,
}

/// 编辑器没有外部目标。这里只演示提示和本档浏览，其他动作均留给实际触发。
fn preview_action(
    action: &ActionSpec,
    catalog: &omnitrigger_actions::PackageCatalog,
) -> PreviewAction {
    match action {
        ActionSpec::Notification => PreviewAction::Notification,
        ActionSpec::Invoke(call)
            if matches!(
                crate::action_packages::clipboard_route(true, call, catalog),
                Ok(crate::action_packages::ClipboardRoute::WorkspaceHistory)
            ) =>
        {
            PreviewAction::BrowseHistory
        }
        _ => PreviewAction::Describe,
    }
}

/// 尚无动作的圈首次放入槽位时使用的方向数。
const DEFAULT_SECTORS: u8 = 8;

/// 格子面板中的常用动作，点一下即放入。
pub(super) enum Preset {
    Keys(&'static str),
    Window(WindowOperation),
}

pub(super) const WHEEL_PRESETS: [(&str, Preset); 12] = [
    ("复制", Preset::Keys("Ctrl+C")),
    ("粘贴", Preset::Keys("Ctrl+V")),
    ("撤销", Preset::Keys("Ctrl+Z")),
    ("全选", Preset::Keys("Ctrl+A")),
    ("关闭标签", Preset::Keys("Ctrl+W")),
    ("新标签", Preset::Keys("Ctrl+T")),
    ("后退", Preset::Keys("Alt+Left")),
    ("前进", Preset::Keys("Alt+Right")),
    ("截图", Preset::Keys("Win+Shift+S")),
    ("最小化", Preset::Window(WindowOperation::Minimize)),
    (
        "最大化 / 恢复",
        Preset::Window(WindowOperation::ToggleMaximize),
    ),
    ("关闭窗口", Preset::Window(WindowOperation::Close)),
];

pub(super) fn preset_action(preset: &Preset) -> ActionSpec {
    match preset {
        Preset::Keys(keys) => {
            ActionSpec::SendShortcut(ShortcutAction::parse(keys).expect("常用快捷键可以解析"))
        }
        Preset::Window(operation) => ActionSpec::Window(WindowAction {
            operation: *operation,
        }),
    }
}

/// 圈的方向数与半格偏移由该圈已有槽位决定；空圈返回 `None`。
pub(super) fn ring_geometry(wheel: &WheelDefinition, ring: u8) -> Option<(u8, bool)> {
    wheel
        .slots
        .iter()
        .filter_map(|slot| slot.position)
        .find(|position| position.ring == ring)
        .map(|position| (position.sectors, position.half_step))
}

/// 三圈的显示几何；空圈按默认方向数显示可放置的空格。
pub(super) fn view_geometry(view: &WheelDefinition) -> RingGeometry {
    std::array::from_fn(|ring| ring_geometry(view, ring as u8).unwrap_or((DEFAULT_SECTORS, false)))
}

/// 允许保留仍可编辑的空格；缩减格数后已经消失的方位不应打开添加面板。
fn retained_cell(view: &WheelDefinition, cell: Option<(u8, u8)>) -> Option<(u8, u8)> {
    cell.filter(|&(ring, sector)| {
        view_geometry(view)
            .get(usize::from(ring))
            .is_some_and(|(sectors, _)| sector < *sectors)
    })
}

/// 返回占用该方位的槽位序号。
pub(super) fn slot_at(wheel: &WheelDefinition, ring: u8, sector: u8) -> Option<usize> {
    wheel.slots.iter().position(|slot| {
        slot.position
            .is_some_and(|position| position.ring == ring && position.sector == sector)
    })
}

/// 编辑器中除“通用”外的范围：旧版整组应用覆盖，或引用场景的专属槽位。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum WheelScope {
    App(String),
    Scene(String),
}

impl WheelScope {
    pub(super) fn label(&self) -> String {
        match self {
            Self::App(process) => process.clone(),
            Self::Scene(name) => format!("场景 {name}"),
        }
    }
}

fn shared(slots: &[WheelSlot]) -> Vec<WheelSlot> {
    slots
        .iter()
        .filter(|slot| slot.scene.is_none())
        .cloned()
        .collect()
}

fn same_cell(left: &WheelSlot, right: &WheelSlot) -> bool {
    left.position
        .zip(right.position)
        .is_some_and(|(left, right)| (left.ring, left.sector) == (right.ring, right.sector))
}

/// 编辑视图会筛选和重排槽位；用所选方位和所属场景找到原配置里的动作。
fn editor_slot_key(
    wheel: &WheelDefinition,
    scope: Option<&WheelScope>,
    slot: &WheelSlot,
) -> Option<crate::wheel_scenes::WheelSlotKey> {
    let application = match scope {
        Some(WheelScope::App(process)) => wheel.override_for_process(Some(process)),
        _ => None,
    };
    let slots = application.map_or(wheel.slots.as_slice(), |app| app.slots.as_slice());
    let position = slot.position?;
    let index = if slots.first()?.position.is_none() {
        // 均分布局在编辑器中按原顺序显示在内圈。
        usize::from(position.sector)
    } else {
        slots
            .iter()
            .position(|source| source.scene == slot.scene && same_cell(source, slot))?
    };
    slots.get(index)?;
    Some(crate::wheel_scenes::WheelSlotKey::new(
        wheel,
        application.map(|app| app.process.as_str()),
        index,
    ))
}

/// 编辑时取出一组槽位，均分布局转为方位布局：
/// “通用”与应用覆盖只含通用槽位；场景视图与该场景下实际显示的一致——
/// 该场景的槽位，加上未被覆盖的通用槽位和必然同时命中的其他场景槽位（沿用的槽位），
/// 写回时按 `scene` 区分沿用与专属。
pub(super) fn scope_view(
    wheel: &WheelDefinition,
    scope: Option<&WheelScope>,
    scenes: &CompiledScenes,
) -> WheelDefinition {
    let mut view = wheel.clone();
    view.app_overrides.clear();
    match scope {
        Some(WheelScope::App(process)) => {
            let slots = wheel
                .app_overrides
                .iter()
                .find(|application| &application.process == process)
                .map_or(&wheel.slots, |application| &application.slots);
            view.slots = shared(slots);
        }
        Some(WheelScope::Scene(name)) => {
            view.slots = shared(&wheel.slots);
            convert_to_positions(&mut view);
            let this = scene_scope(scenes, name).ok();
            let mut inherited: Vec<(u8, WheelSlot)> =
                view.slots.drain(..).map(|slot| (0, slot)).collect();
            for slot in &wheel.slots {
                let Some(other) = slot.scene.as_deref().filter(|other| *other != name) else {
                    continue;
                };
                if let (Some(this), Ok(scope)) = (&this, scene_scope(scenes, other)) {
                    if this.implies(&scope) {
                        inherited.push((scope.specificity(), slot.clone()));
                    }
                }
            }
            let own: Vec<WheelSlot> = wheel
                .slots
                .iter()
                .filter(|slot| slot.scene.as_deref() == Some(name.as_str()))
                .cloned()
                .collect();
            for (index, (specificity, slot)) in inherited.iter().enumerate() {
                let covered = own.iter().any(|scene| same_cell(scene, slot))
                    || inherited
                        .iter()
                        .enumerate()
                        .any(|(other, (better, rival))| {
                            other != index
                                && same_cell(rival, slot)
                                && (better > specificity
                                    || (better == specificity && other < index))
                        });
                if !covered {
                    view.slots.push(slot.clone());
                }
            }
            view.slots.extend(own);
        }
        None => view.slots = shared(&wheel.slots),
    }
    convert_to_positions(&mut view);
    view
}

/// 通用槽位改动后，同组中引用场景的槽位保留原样，并沿用所在圈的新格数与半格偏移。
fn merge_shared(view: Vec<WheelSlot>, previous: &[WheelSlot]) -> Result<Vec<WheelSlot>, String> {
    let mut slots = view;
    let shared_count = slots.len();
    for slot in previous.iter().filter(|slot| slot.scene.is_some()) {
        let mut slot = slot.clone();
        if let Some(position) = slot.position.as_mut() {
            let ring = usize::from(position.ring);
            if let Some((sectors, half_step)) = slots[..shared_count]
                .iter()
                .filter_map(|slot| slot.position)
                .find(|other| other.ring == position.ring)
                .map(|other| (other.sectors, other.half_step))
            {
                if position.sector >= sectors {
                    return Err(format!(
                        "场景 {} 在{}第 {} 格还有专属动作，先把它移走再减少格数。",
                        slot.scene.as_deref().unwrap_or_default(),
                        RING_NAMES[ring],
                        position.sector + 1
                    ));
                }
                position.sectors = sectors;
                position.half_step = half_step;
            }
        }
        slots.push(slot);
    }
    Ok(slots)
}

/// 把编辑后的槽位写回对应的布局。场景视图中改动通用槽位时复制为场景专属槽位；
/// 删除或移走通用槽位、调整格数只能在“通用”中进行，否则拒绝而不替用户改动通用布局。
pub(super) fn store_view(
    wheel: &mut WheelDefinition,
    scope: Option<&WheelScope>,
    view: WheelDefinition,
    scenes: &CompiledScenes,
) -> Result<(), String> {
    match scope {
        Some(WheelScope::App(process)) => {
            let application = wheel
                .app_overrides
                .iter_mut()
                .find(|application| &application.process == process)
                .ok_or("这个应用的专用布局已不存在。")?;
            application.slots = merge_shared(view.slots, &application.slots)?;
        }
        Some(WheelScope::Scene(name)) => {
            let before = scope_view(wheel, scope, scenes);
            for ring in 0..3 {
                if let (Some(old), Some(new)) =
                    (ring_geometry(&before, ring), ring_geometry(&view, ring))
                {
                    if old != new {
                        return Err("格数与半格偏移请在“通用”中调整。".into());
                    }
                }
            }
            let mine = |slot: &WheelSlot| slot.scene.as_deref() == Some(name.as_str());
            let inherited: Vec<&WheelSlot> =
                before.slots.iter().filter(|slot| !mine(slot)).collect();
            if inherited
                .iter()
                .any(|slot| !view.slots.iter().any(|other| same_cell(slot, other)))
            {
                return Err(
                    "沿用的动作（通用或其他场景）只能在其所属范围中删除或移动；在此场景中可以替换为专属动作。"
                        .into(),
                );
            }
            let own = view.slots.into_iter().filter_map(|slot| {
                let unchanged = !mine(&slot) && inherited.iter().any(|original| **original == slot);
                (!unchanged).then(|| WheelSlot {
                    scene: Some(name.clone()),
                    ..slot
                })
            });
            convert_to_positions(wheel);
            wheel
                .slots
                .retain(|slot| slot.scene.as_deref() != Some(name.as_str()));
            wheel.slots.extend(own);
        }
        None => wheel.slots = merge_shared(view.slots, &wheel.slots)?,
    }
    Ok(())
}

/// 在格子上放入动作：已有动作时替换名称与动作，空格新增槽位并沿用该圈几何。
pub(super) fn put_action(
    view: &mut WheelDefinition,
    (ring, sector): (u8, u8),
    label: String,
    action: ActionSpec,
) -> Result<(), String> {
    if let Some(slot) = slot_at(view, ring, sector) {
        view.slots[slot].label = label;
        view.slots[slot].action = action;
        return Ok(());
    }
    if view.slots.len() >= 48 {
        return Err("每组轮盘最多放 48 个动作。".into());
    }
    let (sectors, half_step) = ring_geometry(view, ring).unwrap_or((DEFAULT_SECTORS, false));
    if sector >= sectors {
        return Err("这个格子已不存在，请重新选择。".into());
    }
    view.slots.push(WheelSlot {
        label,
        action,
        position: Some(WheelPosition {
            ring,
            sector,
            sectors,
            half_step,
        }),
        scene: None,
    });
    Ok(())
}

/// 移除格子上的动作；每组至少保留一个动作。
pub(super) fn remove_at(
    view: &mut WheelDefinition,
    (ring, sector): (u8, u8),
) -> Result<(), String> {
    let slot = slot_at(view, ring, sector).ok_or("这个格子是空的。")?;
    if view.slots.len() <= 1 {
        return Err("轮盘至少保留一个动作。".into());
    }
    view.slots.remove(slot);
    Ok(())
}

/// 把槽位移到指定圈与方向；目标方向已有动作时两者互换，空圈按默认 8 个方向启用。
pub(super) fn place_slot(
    wheel: &mut WheelDefinition,
    slot: usize,
    ring: u8,
    sector: u8,
) -> Result<(), String> {
    let current = wheel
        .slots
        .get(slot)
        .and_then(|entry| entry.position)
        .ok_or("要移动的动作已不存在。")?;
    let (sectors, half_step) = ring_geometry(wheel, ring).unwrap_or((DEFAULT_SECTORS, false));
    if sector >= sectors {
        return Err("这个格子已不存在，请重新选择。".into());
    }
    match slot_at(wheel, ring, sector) {
        Some(other) if other == slot => return Ok(()),
        Some(other) => wheel.slots[other].position = Some(current),
        None => {}
    }
    wheel.slots[slot].position = Some(WheelPosition {
        ring,
        sector,
        sectors,
        half_step,
    });
    Ok(())
}

/// 调整整圈的方向数与半格偏移；已有动作会超出新方向数时拒绝，不替用户挪动布局。
pub(super) fn set_ring_layout(
    wheel: &mut WheelDefinition,
    ring: u8,
    sectors: u8,
    half_step: bool,
) -> Result<(), String> {
    let name = RING_NAMES.get(usize::from(ring)).ok_or("圈号应为 0–2。")?;
    if !(1..=16).contains(&sectors) {
        return Err("每圈格数应为 1–16。".into());
    }
    if wheel
        .slots
        .iter()
        .filter_map(|slot| slot.position)
        .any(|position| position.ring == ring && position.sector >= sectors)
    {
        return Err(format!("{name}最后几格还有动作，先把它们移走再减少格数。"));
    }
    for position in wheel
        .slots
        .iter_mut()
        .filter_map(|slot| slot.position.as_mut())
        .filter(|position| position.ring == ring)
    {
        position.sectors = sectors;
        position.half_step = half_step;
    }
    Ok(())
}

/// 均分布局转为内圈方位布局，方向与原均分位置一致；此后删除槽位不再重排其他动作。
pub(super) fn convert_to_positions(wheel: &mut WheelDefinition) {
    if wheel.slots.iter().any(|slot| slot.position.is_some()) {
        return;
    }
    let sectors = wheel.slots.len() as u8;
    for (index, slot) in wheel.slots.iter_mut().enumerate() {
        slot.position = Some(WheelPosition {
            ring: 0,
            sector: index as u8,
            sectors,
            half_step: false,
        });
    }
}

impl Workspace {
    fn selected_wheel_index(&self) -> Option<usize> {
        self.wheel_selected
            .as_ref()
            .and_then(|id| self.wheels.wheels.iter().position(|wheel| &wheel.id == id))
    }

    pub(super) fn selected_wheel_runtime_id(&self) -> Option<u64> {
        let definition = self.wheels.wheels.get(self.selected_wheel_index()?)?;
        let view = self.wheel_view()?;
        let slot = view.slots.get(self.wheel_slot_selected?)?;
        let key = editor_slot_key(definition, self.wheel_scope_for(definition).as_ref(), slot)?;
        self.wheel_runtime_ids.get(&key).copied()
    }

    fn commit_wheels(&mut self, proposed: Wheels, cx: &mut Context<Self>) -> bool {
        if let Some(error) = &self.wheels_error {
            self.error = Some(format!("原轮盘配置不可用，请先修复文件：{error}"));
            cx.notify();
            return false;
        }
        if let Err(error) = self.check_wheel_scenes(&proposed) {
            self.error = Some(error);
            cx.notify();
            return false;
        }
        match proposed.save_changes(&crate::settings::path(), &self.wheels, &self.wheel_scenes) {
            Ok(()) => {
                self.replace_wheels(proposed, cx);
                self.sync_bindings();
                self.status = "轮盘已保存。".into();
                cx.notify();
                true
            }
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                false
            }
        }
    }

    /// 槽位引用的场景须在 bindings.json 中定义且只限定前台应用；场景读取失败时说明原因。
    fn check_wheel_scenes(&self, wheels: &Wheels) -> Result<(), String> {
        for wheel in &wheels.wheels {
            crate::wheel_scenes::check_wheel(wheel, &self.wheel_scenes).map_err(|error| {
                let reason = match &self.wheel_scenes_error {
                    Some(load) => format!("（场景定义读取失败：{load}）"),
                    None => String::new(),
                };
                format!("轮盘“{}”：{error}{reason}", wheel.name)
            })?;
        }
        Ok(())
    }

    pub(super) fn replace_wheels(&mut self, wheels: Wheels, cx: &mut Context<Self>) {
        self.hide_wheel(cx);
        self.wheel_preview = None;
        remap_wheel_runtime_ids(&self.wheels, &wheels, &mut self.wheel_runtime_ids);
        self.wheels = wheels;
        self.wheels_error = None;
        self.error = None;
    }

    /// 按同一批磁盘配置刷新轮盘及其呼出热键，只有成功后才恢复编辑位置。
    pub(super) fn reload_wheels(&mut self, cx: &mut Context<Self>) {
        if self.recording {
            return;
        }
        let selected = self.wheel_selected.clone();
        let scope = self.wheel_scope.clone();
        let cell = self.wheel_cell;
        match self.reload_wheel_and_binding_snapshot(cx) {
            Ok(()) => {
                if let Some(wheel) = self
                    .wheels
                    .wheels
                    .iter()
                    .find(|wheel| Some(&wheel.id) == selected.as_ref())
                {
                    let kept_scope = scope
                        .as_ref()
                        .is_none_or(|scope| self.wheel_scope_choices(wheel).contains(scope));
                    let id = wheel.id.clone();
                    self.select_wheel(id, cx);
                    if kept_scope {
                        self.wheel_scope = scope;
                        self.wheel_cell = self
                            .wheel_view()
                            .and_then(|view| retained_cell(&view, cell));
                        self.load_wheel_cell(cx);
                    }
                } else if let Some(first) = self.wheels.wheels.first() {
                    self.select_wheel(first.id.clone(), cx);
                } else {
                    self.wheel_selected = None;
                    self.wheel_scope = None;
                    self.wheel_cell = None;
                    self.wheel_hover = None;
                    self.wheel_moving = false;
                    self.wheel_replacing = false;
                    self.wheel_scope_open = false;
                    self.wheel_source_binding = None;
                    self.load_wheel_cell(cx);
                }
                self.status = "已重新载入轮盘和热键配置。".into();
            }
            // 热键草稿冲突已有可操作的横幅，显式重新载入后会清除。
            Err(error) if self.binding_reload_notice.as_ref() == Some(&error) => {}
            Err(error) => {
                if self.wheels_error.is_some() {
                    self.wheels_error = Some(error.clone());
                }
                self.error = Some(format!("重新载入失败，仍使用当前轮盘：{error}"));
            }
        }
        cx.notify();
    }

    /// 用系统中 .json 的默认程序打开轮盘配置，未关联程序时改用记事本；修改后需重新载入。
    pub(super) fn open_wheels_file(&mut self, cx: &mut Context<Self>) {
        let path = crate::settings::path().with_file_name("wheels.json");
        let path = std::path::absolute(&path).unwrap_or(path);
        if let Err(error) = self.wheels.prepare_file(&crate::settings::path()) {
            self.error = Some(error);
            cx.notify();
            return;
        }
        let target = path.to_string_lossy().into_owned();
        std::thread::spawn(move || {
            if omnitrigger_windows::open_target(&target).is_err() {
                let _ = std::process::Command::new("notepad.exe")
                    .arg(&target)
                    .spawn();
            }
        });
        self.status = "已打开 wheels.json，保存文件后点击“重新载入”。".into();
        cx.notify();
    }

    /// 编辑器可选的范围：旧版应用覆盖，以及只限定前台应用的场景和槽位已引用的场景。
    pub(super) fn wheel_scope_choices(&self, wheel: &WheelDefinition) -> Vec<WheelScope> {
        let mut scenes: Vec<String> = self
            .wheel_scenes
            .iter()
            .filter(|(_, scene)| scene.pointer.is_global() && !scene.foreground.is_global())
            .map(|(name, _)| name.to_string())
            .collect();
        for name in wheel.slots.iter().filter_map(|slot| slot.scene.as_ref()) {
            if !scenes.contains(name) {
                scenes.push(name.clone());
            }
        }
        wheel
            .app_overrides
            .iter()
            .map(|application| WheelScope::App(application.process.clone()))
            .chain(scenes.into_iter().map(WheelScope::Scene))
            .collect()
    }

    /// 当前编辑的范围；所选应用或场景已不可选时回到通用布局。
    pub(super) fn wheel_scope_for(&self, wheel: &WheelDefinition) -> Option<WheelScope> {
        self.wheel_scope
            .clone()
            .filter(|scope| self.wheel_scope_choices(wheel).contains(scope))
    }

    /// 选中轮盘在当前范围下的槽位，均分布局已转为方位布局。
    pub(super) fn wheel_view(&self) -> Option<WheelDefinition> {
        let wheel = self.wheels.wheels.get(self.selected_wheel_index()?)?;
        Some(scope_view(
            wheel,
            self.wheel_scope_for(wheel).as_ref(),
            &self.wheel_scenes,
        ))
    }

    /// 修改当前范围的槽位并立即保存；成功后按选中格子刷新面板。
    fn edit_wheel_view(
        &mut self,
        status: impl Into<String>,
        cx: &mut Context<Self>,
        edit: impl FnOnce(&mut WheelDefinition) -> Result<(), String>,
    ) -> bool {
        let Some(index) = self.selected_wheel_index() else {
            return false;
        };
        let mut proposed = self.wheels.clone();
        let wheel = &mut proposed.wheels[index];
        let scope = self.wheel_scope_for(wheel);
        let mut view = scope_view(wheel, scope.as_ref(), &self.wheel_scenes);
        if let Err(error) = edit(&mut view)
            .and_then(|()| store_view(wheel, scope.as_ref(), view, &self.wheel_scenes))
        {
            self.error = Some(error);
            cx.notify();
            return false;
        }
        if !self.commit_wheels(proposed, cx) {
            return false;
        }
        self.status = status.into();
        self.load_wheel_cell(cx);
        true
    }

    /// 按选中格子同步槽位序号与面板输入。
    fn load_wheel_cell(&mut self, cx: &mut Context<Self>) {
        let slot = self
            .wheel_view()
            .zip(self.wheel_cell)
            .and_then(|(view, (ring, sector))| {
                slot_at(&view, ring, sector).map(|index| (index, view.slots[index].clone()))
            });
        self.wheel_slot_selected = slot.as_ref().map(|(index, _)| *index);
        let label = slot
            .as_ref()
            .map(|(_, slot)| slot.label.clone())
            .unwrap_or_default();
        self.wheel_slot_label
            .update(cx, |input, cx| input.set_text(label, cx));
        if let Some((_, slot)) = &slot {
            if let ActionSpec::Local(action) = &slot.action {
                self.wheel_local_editor.load(action, cx);
            }
        }
        self.wheel_slot_shortcut
            .update(cx, |input, cx| input.set_text("", cx));
    }

    /// 离开格子前保存名称。写入失败时由调用方保留选中项和输入框。
    fn flush_wheel_label(&mut self, cx: &mut Context<Self>) -> bool {
        let Some((view, (ring, sector))) = self.wheel_view().zip(self.wheel_cell) else {
            return true;
        };
        let Some(slot) = slot_at(&view, ring, sector) else {
            return true;
        };
        let label = self.wheel_slot_label.read(cx).text().trim().to_string();
        if label.is_empty() || label == view.slots[slot].label {
            return true;
        }
        self.edit_wheel_view("名称已保存。", cx, |view| {
            view.slots[slot].label = label;
            Ok(())
        })
    }

    /// 点格子：移动中则把动作移到这里（有动作就互换），否则选中或取消选中。
    pub(super) fn select_wheel_cell(&mut self, cell: (u8, u8), cx: &mut Context<Self>) {
        if self.wheel_moving {
            if let Some(from) = self.wheel_cell.filter(|from| *from != cell) {
                if !self.edit_wheel_view("已移动。", cx, |view| {
                    let slot = slot_at(view, from.0, from.1).ok_or("要移动的动作已不存在。")?;
                    place_slot(view, slot, cell.0, cell.1)
                }) {
                    return;
                }
                self.wheel_cell = Some(cell);
                self.load_wheel_cell(cx);
            }
            self.wheel_moving = false;
            cx.notify();
            return;
        }
        if !self.flush_wheel_label(cx) {
            return;
        }
        self.wheel_cell = (self.wheel_cell != Some(cell)).then_some(cell);
        self.wheel_replacing = false;
        self.wheel_source_binding = None;
        self.load_wheel_cell(cx);
        cx.notify();
    }

    pub(super) fn deselect_wheel_cell(&mut self, cx: &mut Context<Self>) {
        if !self.flush_wheel_label(cx) {
            return;
        }
        self.wheel_cell = None;
        self.wheel_moving = false;
        self.wheel_replacing = false;
        self.load_wheel_cell(cx);
        cx.notify();
    }

    pub(super) fn set_wheel_moving(&mut self, moving: bool, cx: &mut Context<Self>) {
        if !self.flush_wheel_label(cx) {
            return;
        }
        self.wheel_moving = moving;
        cx.notify();
    }

    pub(super) fn set_wheel_replacing(&mut self, replacing: bool, cx: &mut Context<Self>) {
        if !self.flush_wheel_label(cx) {
            return;
        }
        self.wheel_replacing = replacing;
        self.wheel_source_binding = None;
        cx.notify();
    }

    pub(super) fn set_wheel_scope(&mut self, scope: Option<WheelScope>, cx: &mut Context<Self>) {
        if !self.flush_wheel_label(cx) {
            return;
        }
        self.wheel_scope = scope;
        self.wheel_scope_open = false;
        self.wheel_cell = None;
        self.wheel_moving = false;
        self.wheel_replacing = false;
        self.load_wheel_cell(cx);
        cx.notify();
    }

    fn put_wheel_action(&mut self, label: String, action: ActionSpec, cx: &mut Context<Self>) {
        let Some(cell) = self.wheel_cell else {
            return;
        };
        let status = format!("已放入“{label}”。");
        if self.edit_wheel_view(status, cx, |view| put_action(view, cell, label, action)) {
            self.wheel_replacing = false;
        }
    }

    pub(super) fn put_wheel_preset(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some((label, preset)) = WHEEL_PRESETS.get(index) {
            self.put_wheel_action((*label).into(), preset_action(preset), cx);
        }
    }

    /// 自定义快捷键以按键写法作为初始名称，放入后可在面板里改名。
    pub(super) fn put_wheel_shortcut(&mut self, cx: &mut Context<Self>) {
        match ShortcutAction::parse(&self.wheel_slot_shortcut.read(cx).text()) {
            Ok(shortcut) => {
                let label = shortcut.display().chars().take(30).collect();
                self.put_wheel_action(label, ActionSpec::SendShortcut(shortcut), cx);
            }
            Err(error) => {
                self.error = Some(error);
                cx.notify();
            }
        }
    }

    pub(super) fn put_wheel_local_action(&mut self, cx: &mut Context<Self>) {
        self.put_wheel_action(
            "本地动作".into(),
            ActionSpec::Local(crate::local_editor::initial_action()),
            cx,
        );
    }

    /// 复制已有触发器的动作；之后与原触发器互不影响。
    pub(super) fn put_wheel_binding(&mut self, binding_id: u64, cx: &mut Context<Self>) {
        let Some(binding) = self.binding(binding_id).cloned() else {
            return;
        };
        if matches!(
            binding.action,
            ActionSpec::ShowPanel | ActionSpec::ShowWheel(_)
        ) {
            return;
        }
        let label: String = binding.message.trim().chars().take(30).collect();
        let label = if label.is_empty() {
            "动作".into()
        } else {
            label
        };
        self.put_wheel_action(label, binding.action, cx);
    }

    pub(super) fn save_wheel_local(&mut self, cx: &mut Context<Self>) {
        let Some((ring, sector)) = self.wheel_cell else {
            return;
        };
        let action = match self.wheel_local_editor.snapshot(cx) {
            Ok(action) => action,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        let label = self.wheel_slot_label.read(cx).text().trim().to_owned();
        self.edit_wheel_view("本地动作已保存。", cx, |view| {
            let slot = slot_at(view, ring, sector).ok_or("这个格子是空的。")?;
            if !label.is_empty() {
                view.slots[slot].label = label;
            }
            view.slots[slot].action = ActionSpec::Local(action);
            Ok(())
        });
    }

    pub(super) fn remove_wheel_cell(&mut self, cx: &mut Context<Self>) {
        if let Some(cell) = self.wheel_cell {
            self.edit_wheel_view("已移除。", cx, |view| remove_at(view, cell));
        }
    }

    pub(super) fn resize_wheel_ring(&mut self, ring: u8, delta: i8, cx: &mut Context<Self>) {
        let Some((sectors, half_step)) = self
            .wheel_view()
            .and_then(|view| ring_geometry(&view, ring))
        else {
            return;
        };
        let Some(next) = sectors
            .checked_add_signed(delta)
            .filter(|next| (2..=16).contains(next))
        else {
            return;
        };
        let status = format!("{}改为 {next} 格。", RING_NAMES[usize::from(ring)]);
        self.edit_wheel_view(status, cx, |view| {
            set_ring_layout(view, ring, next, half_step)
        });
    }

    /// 切换轮盘前先保存正在编辑的名称。
    pub(super) fn switch_wheel(&mut self, id: String, cx: &mut Context<Self>) {
        if self.flush_wheel_label(cx) {
            self.select_wheel(id, cx);
        }
    }

    pub(super) fn select_wheel(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(wheel) = self.wheels.wheels.iter().find(|wheel| wheel.id == id) else {
            return;
        };
        let name = wheel.name.clone();
        self.wheel_preview = None;
        self.smooth_scroll.cancel("wheel-editor-content");
        self.smooth_scroll.cancel("wheel-source-list");
        self.wheel_selected = Some(id);
        self.wheel_cell = None;
        self.wheel_moving = false;
        self.wheel_replacing = false;
        self.wheel_scope = None;
        self.wheel_scope_open = false;
        self.wheel_hover = None;
        self.wheel_source_binding = None;
        self.wheel_name
            .update(cx, |input, cx| input.set_text(name, cx));
        self.load_wheel_cell(cx);
        cx.notify();
    }

    pub(super) fn create_wheel(&mut self, cx: &mut Context<Self>) {
        if self.wheels.wheels.len() >= 16 {
            self.error = Some("最多可配置 16 个轮盘。".into());
            cx.notify();
            return;
        }
        if !self.flush_wheel_label(cx) {
            return;
        }
        let number = (1..)
            .find(|number| {
                !self
                    .wheels
                    .wheels
                    .iter()
                    .any(|wheel| wheel.id == format!("orbit-{number}"))
            })
            .unwrap();
        let mut definition = Wheels::default().wheels.remove(0);
        definition.id = format!("orbit-{number}");
        definition.name = format!("新轮盘 {number}");
        let id = definition.id.clone();
        let mut proposed = self.wheels.clone();
        proposed.wheels.push(definition);
        if self.commit_wheels(proposed, cx) {
            self.select_wheel(id, cx);
        }
    }

    pub(super) fn save_wheel_name(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.selected_wheel_index() else {
            return;
        };
        let mut proposed = self.wheels.clone();
        proposed.wheels[index].name = self.wheel_name.read(cx).text().trim().into();
        self.commit_wheels(proposed, cx);
    }

    pub(super) fn set_wheel_accent(&mut self, accent: WheelAccent, cx: &mut Context<Self>) {
        let Some(index) = self.selected_wheel_index() else {
            return;
        };
        let mut proposed = self.wheels.clone();
        proposed.wheels[index].accent = accent;
        self.commit_wheels(proposed, cx);
    }

    pub(super) fn remove_wheel(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.selected_wheel_index() else {
            return;
        };
        let id = self.wheels.wheels[index].id.clone();
        if self.triggers.bindings().iter().any(|binding| matches!(&binding.action, ActionSpec::ShowWheel(reference) if reference.wheel_id == id)) {
            self.error = Some("此轮盘仍被触发器引用，请先删除或更改对应呼出绑定。".into());
            cx.notify(); return;
        }
        let mut proposed = self.wheels.clone();
        proposed.wheels.remove(index);
        if self.commit_wheels(proposed, cx) {
            self.wheel_selected = None;
            self.wheel_cell = None;
            if let Some(wheel) = self.wheels.wheels.first() {
                self.select_wheel(wheel.id.clone(), cx);
            }
        }
    }

    pub(super) fn choose_draft_wheel(&mut self, id: String, cx: &mut Context<Self>) {
        self.draft_action = ActionSpec::ShowWheel(WheelActionRef { wheel_id: id });
        cx.notify();
    }

    pub(super) fn bind_wheel(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.selected_wheel_index() else {
            return;
        };
        let wheel = self.wheels.wheels[index].clone();
        if let Some(existing) = wheel_binding(self.triggers.bindings(), &wheel.id) {
            self.select(existing.spec.id, cx);
            return;
        }
        let Some(id) = self.draft_id(self.pending.as_ref().map(|spec| spec.id)) else {
            cx.notify();
            return;
        };
        self.clear_pending();
        self.edited_spec = None;
        let spec = crate::presets::PRESETS
            .iter()
            .find(|preset| preset.key == "right-quick-drag")
            .expect("快速拖拽预设随程序提供")
            .spec(id);
        self.load_trigger_fields(&spec, cx);
        self.conflict = self.check_draft_trigger(&spec, None).err();
        self.pending = Some(spec);
        self.load_action(
            ActionSpec::ShowWheel(WheelActionRef { wheel_id: wheel.id }),
            cx,
        );
        self.message.update(cx, |input, cx| {
            input.set_text(format!("打开{}", wheel.name), cx)
        });
        self.page = Page::Triggers;
        self.confirm_delete = None;
        self.status = "已选择“右键快速拖拽”触发预设，检查参数后保存生效。".into();
        cx.notify();
    }

    pub(super) fn preview_wheel(&mut self, cx: &mut Context<Self>) {
        if self.recording || self.wheels_error.is_some() {
            return;
        }
        if self.test_data && !self.listen {
            self.wheel_preview = Some(WheelPreview::default());
            cx.notify();
        } else if let Some(id) = self.wheel_selected.clone() {
            self.open_wheel(&id, cx);
        }
    }

    pub(super) fn preview_wheel_cell(&mut self, cell: (u8, u8), cx: &mut Context<Self>) {
        if self.recording {
            return;
        }
        let Some(view) = self.wheel_view() else {
            return;
        };
        let Some(preview) = &mut self.wheel_preview else {
            return;
        };
        preview.selected = Some(cell);
        if let Some(slot) = preview.selected_slot(&view) {
            match preview_action(&slot.action, self.action_service.catalog()) {
                PreviewAction::Notification => self.show_message(
                    Vec::new(),
                    format!("{} · {}", view.name, slot.label),
                    HitSource::Preview,
                    cx,
                ),
                PreviewAction::BrowseHistory => {
                    self.status = "已从轮盘打开本测试档文本历史。点“轮盘”可返回编辑。".into();
                    self.open_page(Page::TextHistory, cx);
                }
                PreviewAction::Describe => {}
            }
        }
        cx.notify();
    }

    pub(super) fn hide_wheel(&mut self, cx: &mut Context<Self>) {
        if let Some(drag) = self.drag_wheel_session.take() {
            self.send(BackendCommand::CancelDrag {
                session: drag.session,
            });
        }
        self.wheel_generation = self.wheel_generation.wrapping_add(1);
        if let Some(wheel) = self.wheel_window {
            let _ = wheel.update(cx, |view, window, _| view.hide(window));
        }
        self.wheel_active = None;
        self.active_wheel = None;
        self.wheel_target = None;
        if self.wheel_registered.take().is_some() {
            self.send(BackendCommand::SetPanelWindow(None));
        }
    }

    fn hide_panel_for_wheel(&mut self, cx: &mut Context<Self>) {
        self.panel_show_generation = self.panel_show_generation.wrapping_add(1);
        if let Some(panel) = self.panel {
            let _ = panel.update(cx, |view, window, _| view.hide(window));
        }
        if self.panel_registered.take().is_some() {
            self.send(BackendCommand::SetPanelWindow(None));
        }
    }

    fn prepare_wheel(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Result<WindowHandle<RadialWheel>, String> {
        if let Some(wheel) = self.wheel_window {
            return Ok(wheel);
        }
        let requests = self.wheel_sender.clone();
        let wake = self.ui_wake.clone();
        let bounds = Bounds::centered(None, size(px(WHEEL_SIZE), px(WHEEL_SIZE)), cx);
        let mut preparation_error = None;
        let wheel = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: None,
                    kind: WindowKind::PopUp,
                    is_resizable: false,
                    is_minimizable: false,
                    is_movable: false,
                    focus: false,
                    show: true,
                    window_background: WindowBackgroundAppearance::Transparent,
                    app_id: Some("OmniTrigger.Wheel".into()),
                    ..Default::default()
                },
                |window, cx| {
                    match HasWindowHandle::window_handle(&*window).map(|handle| handle.as_raw()) {
                        Ok(RawWindowHandle::Win32(handle)) => {
                            preparation_error =
                                omnitrigger_windows::prepare_wheel_window(handle.hwnd.get()).err();
                        }
                        _ => preparation_error = Some("无法取得轮盘窗口句柄。".into()),
                    }
                    cx.new(|cx| RadialWheel::new(requests, wake, window, cx))
                },
            )
            .map_err(|error| format!("准备轮盘失败：{error}"))?;
        if let Some(error) = preparation_error {
            let _ = wheel.update(cx, |_, window, _| window.remove_window());
            return Err(error);
        }
        self.wheel_window = Some(wheel);
        Ok(wheel)
    }

    fn wheel_slot_states(&self, wheel: &ActiveWheel) -> Vec<Option<String>> {
        wheel
            .definition
            .slots
            .iter()
            .zip(&wheel.runtime_ids)
            .map(|(slot, runtime_id)| {
                if self.paused || !self.ready {
                    return Some("监听尚未就绪或已暂停。".into());
                }
                self.action_unavailable_reason(&slot.action).or_else(|| {
                    self.action_busy(*runtime_id, &slot.action)
                        .then(|| "此动作仍在执行，请稍后重试。".into())
                })
            })
            .collect()
    }

    pub(super) fn open_wheel(&mut self, id: &str, cx: &mut Context<Self>) {
        self.open_wheel_placed(id, None, cx);
    }

    fn open_wheel_placed(
        &mut self,
        id: &str,
        placement: Option<DragPlacement>,
        cx: &mut Context<Self>,
    ) {
        if self.recording {
            self.status = "请先结束录制，再打开轮盘。".into();
            cx.notify();
            return;
        }
        if self.paused || !self.ready {
            self.error = Some("请先恢复输入监听，再打开轮盘。".into());
            cx.notify();
            return;
        }
        if let Some(error) = &self.wheels_error {
            self.error = Some(error.clone());
            cx.notify();
            return;
        }
        let Some(definition) = self
            .wheels
            .wheels
            .iter()
            .find(|wheel| wheel.id == id)
            .cloned()
        else {
            self.error = Some(format!("轮盘“{id}”不存在，请在轮盘页重新选择。"));
            cx.notify();
            return;
        };
        let already_open = self.wheel_active.as_deref() == Some(id)
            && self.wheel_window.is_some_and(|wheel| {
                wheel
                    .update(cx, |view, _, _| view.is_visible())
                    .unwrap_or(false)
            });
        self.hide_wheel(cx);
        self.hide_panel_for_wheel(cx);
        if already_open && placement.is_none() {
            cx.notify();
            return;
        }
        let target = match omnitrigger_windows::capture_shortcut_target() {
            Ok(target) => target,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        let process = omnitrigger_windows::shortcut_target_context(target).ok();
        let (scene_process, role) = omnitrigger_windows::shortcut_target_scene_context(target)
            .map_or(
                (None, omnitrigger_core::WindowRole::Unknown),
                |(process, role)| (Some(process), role),
            );
        let wheel = match ActiveWheel::for_target(
            &definition,
            &self.wheel_scenes,
            process.as_deref(),
            crate::wheel_scenes::TargetContext {
                process: scene_process.as_deref(),
                role,
            },
            &mut self.wheel_runtime_ids,
            &mut self.triggers,
        ) {
            Ok(wheel) => wheel,
            Err(error) => {
                self.error = Some(error.message());
                cx.notify();
                return;
            }
        };
        let wheel_size = crate::radial_wheel::wheel_size(&wheel.definition);
        let window = match self.prepare_wheel(cx) {
            Ok(window) => window,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        let states = self.wheel_slot_states(&wheel);
        let definition = wheel.definition.clone();
        let generation = self.wheel_generation;
        let wheel_id = id.to_string();
        self.active_wheel = Some(wheel);
        let result = window.update(cx, |view, window, cx| {
            view.present(definition, generation, states, window, cx);
            if let Some(drag) = placement {
                view.begin_drag(
                    drag.origin,
                    drag.cancel_radius_px,
                    drag.position,
                    drag.scale,
                    cx,
                );
            }
            match HasWindowHandle::window_handle(&*window).map(|handle| handle.as_raw()) {
                Ok(RawWindowHandle::Win32(handle)) => Some(handle.hwnd.get()),
                _ => None,
            }
        });
        let Ok(Some(handle)) = result else {
            self.error = Some("轮盘窗口不可用。".into());
            self.hide_wheel(cx);
            cx.notify();
            return;
        };
        self.wheel_active = Some(wheel_id);
        self.wheel_target = Some(target);
        // 原生显示可能同步发送窗口消息；在当前 GPUI 状态借用结束后提交。
        cx.spawn(async move |this, cx| {
            let allowed = this
                .update(cx, |this, cx| {
                    this.wheel_generation == generation
                        && this.wheel_active.is_some()
                        && this.wheel_target == Some(target)
                        && this.ready
                        && !this.paused
                        && !this.recording
                        && window
                            .update(cx, |view, _, _| view.is_visible())
                            .unwrap_or(false)
                })
                .unwrap_or(false);
            if !allowed {
                return;
            }
            let shown = if omnitrigger_windows::capture_shortcut_target().ok() != Some(target) {
                Err("前台窗口已切换，已取消打开轮盘。".into())
            } else if let Some(drag) = placement {
                omnitrigger_windows::show_wheel_at_point_with_size(
                    handle,
                    drag.origin.0,
                    drag.origin.1,
                    wheel_size,
                    wheel_size,
                )
            } else {
                omnitrigger_windows::show_wheel_at_cursor_with_size(handle, wheel_size, wheel_size)
            };
            let should_hide = this
                .update(cx, |this, cx| {
                    if this.wheel_generation != generation {
                        return window
                            .update(cx, |view, _, _| !view.is_visible())
                            .unwrap_or(true);
                    }
                    match shown {
                        Ok(())
                            if omnitrigger_windows::capture_shortcut_target().ok()
                                == Some(target) =>
                        {
                            this.wheel_registered = Some(handle);
                            this.send(BackendCommand::SetPanelWindow(Some(handle)));
                            this.status = if placement.is_some() {
                                "快速拖拽已认领：按住移动选择，松开鼠标键执行，回到起点取消。"
                            } else {
                                "轨道轮盘已打开：移动选择后点击，或用数字键执行。"
                            }
                            .into();
                        }
                        Ok(()) => {
                            this.hide_wheel(cx);
                        }
                        Err(error) => {
                            this.error = Some(error);
                            this.hide_wheel(cx);
                        }
                    }
                    cx.notify();
                    this.wheel_active.is_none()
                })
                .unwrap_or(true);
            if should_hide {
                omnitrigger_windows::hide_panel_window(handle);
            }
        })
        .detach();
        cx.notify();
    }

    pub(super) fn refresh_wheel(&self, cx: &mut Context<Self>) {
        if let Some(window) = self.wheel_window {
            if !window
                .update(cx, |view, _, _| view.is_visible())
                .unwrap_or(false)
            {
                return;
            }
            if let Some(wheel) = &self.active_wheel {
                let states = self.wheel_slot_states(wheel);
                let _ = window.update(cx, |view, _, cx| view.update_states(states, cx));
            }
        }
    }

    pub(super) fn handle_drag_event(
        &mut self,
        event: DragEvent,
        foreground: isize,
        cx: &mut Context<Self>,
    ) {
        match event.phase {
            DragPhase::Started => {
                let Some(binding) = self.binding(event.id).cloned() else {
                    return;
                };
                let ActionSpec::ShowWheel(reference) = &binding.action else {
                    return;
                };
                let Activation::Drag {
                    min_distance_px, ..
                } = binding.spec.activation
                else {
                    return;
                };
                if !binding.enabled
                    || self.paused
                    || self.recording
                    || !self.ready
                    || !omnitrigger_windows::is_foreground_window(foreground)
                {
                    self.send(BackendCommand::CancelDrag {
                        session: event.session,
                    });
                    return;
                }
                let scale = match omnitrigger_windows::pointer_scale_at_point(
                    event.origin.0,
                    event.origin.1,
                ) {
                    Ok(scale) => scale,
                    Err(error) => {
                        self.error = Some(error);
                        self.send(BackendCommand::CancelDrag {
                            session: event.session,
                        });
                        cx.notify();
                        return;
                    }
                };
                let placement = DragPlacement {
                    origin: event.origin,
                    position: event.position,
                    cancel_radius_px: (min_distance_px / 2).max(1),
                    scale,
                };
                self.open_wheel_placed(&reference.wheel_id, Some(placement), cx);
                if self.wheel_active.as_deref() != Some(reference.wheel_id.as_str()) {
                    self.send(BackendCommand::CancelDrag {
                        session: event.session,
                    });
                    return;
                }
                let Some(target) = self.wheel_target else {
                    return;
                };
                let Some(wheel) = self.active_wheel.clone() else {
                    return;
                };
                self.drag_wheel_session = Some(DragWheelSession {
                    session: event.session,
                    binding_id: event.id,
                    wheel,
                    origin: event.origin,
                    cancel_radius_px: placement.cancel_radius_px,
                    scale: placement.scale,
                    target,
                });
                let generation = self.wheel_generation;
                cx.spawn(async move |this, cx| loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(16))
                        .await;
                    let alive = this
                        .update(cx, |this, cx| {
                            if this.wheel_generation != generation
                                || this
                                    .drag_wheel_session
                                    .as_ref()
                                    .is_none_or(|drag| drag.session != event.session)
                            {
                                return false;
                            }
                            if let (Some(window), Some(position)) = (
                                this.wheel_window,
                                this.backend
                                    .as_ref()
                                    .and_then(|backend| backend.pointer_position()),
                            ) {
                                let _ =
                                    window.update(cx, |view, _, cx| view.update_drag(position, cx));
                            }
                            true
                        })
                        .unwrap_or(false);
                    if !alive {
                        break;
                    }
                })
                .detach();
            }
            DragPhase::Ended | DragPhase::Cancelled => {
                if !self.drag_wheel_session.as_ref().is_some_and(|drag| {
                    drag.session == event.session && drag.binding_id == event.id
                }) {
                    return;
                }
                let drag = self.drag_wheel_session.take().expect("会话已核验");
                let target_unchanged = omnitrigger_windows::capture_shortcut_target().ok()
                    == Some(drag.target)
                    && omnitrigger_windows::is_foreground_window(foreground);
                let still_visible = self.wheel_window.is_some_and(|window| {
                    window
                        .update(cx, |view, _, _| view.is_visible())
                        .unwrap_or(false)
                });
                // Ended 即刻按最终位置决策；即便快速甩动先于首帧显示，也不丢失完整会话。
                self.hide_wheel(cx);
                if event.phase == DragPhase::Cancelled
                    || !still_visible
                    || self.paused
                    || self.recording
                    || !self.ready
                    || !target_unchanged
                {
                    self.status = "拖拽轮盘已取消。".into();
                    cx.notify();
                    return;
                }
                let selected = crate::radial_wheel::drag_slot_for_definition(
                    drag.origin,
                    event.position,
                    &drag.wheel.definition,
                    drag.cancel_radius_px,
                    drag.scale,
                );
                if let Some(index) = selected {
                    if let Some((id, name, entry)) = drag.wheel.selection(index) {
                        self.execute_wheel_entry(id, name, entry, drag.target, cx);
                    }
                } else {
                    self.status = "已回到起点，取消轮盘。".into();
                }
                cx.notify();
            }
        }
    }

    fn execute_wheel_entry(
        &mut self,
        id: u64,
        wheel_name: String,
        entry: WheelSlot,
        target: omnitrigger_windows::ShortcutTarget,
        cx: &mut Context<Self>,
    ) {
        if let Some(reason) = self.action_unavailable_reason(&entry.action) {
            self.error = Some(reason);
            return;
        }
        if self.action_busy(id, &entry.action) {
            self.error = Some("此轮盘动作仍在执行，请稍后重试。".into());
            return;
        }
        let label = format!("{wheel_name} · {}", entry.label);
        if matches!(entry.action, ActionSpec::Notification) {
            self.show_message(Vec::new(), label, HitSource::Wheel, cx);
        } else {
            self.record_hit(Vec::new(), label, HitSource::Wheel);
            self.dispatch_action_to(id, entry.action, Some(target), cx);
        }
    }

    pub(super) fn open_wheel_after_launch(
        &mut self,
        id: String,
        target: omnitrigger_windows::ShortcutTarget,
        cx: &mut Context<Self>,
    ) {
        let generation = self.wheel_generation;
        let main_handle = self
            .main_window
            .update(cx, |_, window, _| {
                HasWindowHandle::window_handle(&*window)
                    .ok()
                    .and_then(|handle| match handle.as_raw() {
                        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
                        _ => None,
                    })
            })
            .ok()
            .flatten();
        cx.spawn(async move |this, cx| {
            let allowed = this
                .update(cx, |this, _| {
                    this.wheel_generation == generation && !this.recording && !this.paused
                })
                .unwrap_or(false);
            if !allowed {
                return;
            }
            let still_original =
                omnitrigger_windows::capture_shortcut_target().ok() == Some(target);
            let startup_activated_main =
                main_handle.is_some_and(omnitrigger_windows::is_foreground_window);
            // 只收尾本程序启动时的隐式激活；用户已切到其他窗口时取消，不再次抢前台。
            let restored = if still_original {
                Ok(true)
            } else if startup_activated_main {
                omnitrigger_windows::restore_shortcut_target(target)
            } else {
                Ok(false)
            };
            let _ = this.update(cx, |this, cx| {
                if this.wheel_generation != generation {
                    return;
                }
                match restored {
                    Ok(true)
                        if omnitrigger_windows::capture_shortcut_target().ok() == Some(target) =>
                    {
                        this.open_wheel(&id, cx)
                    }
                    Ok(_) => {
                        this.status =
                            "启动时前台已变化，已取消自动打开轮盘；可用呼出组合重试。".into();
                    }
                    Err(error) => {
                        this.error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn poll_wheel(&mut self, cx: &mut Context<Self>) -> bool {
        let mut changed = false;
        for _ in 0..8 {
            let Ok(request) = self.wheel_requests.try_recv() else {
                break;
            };
            changed = true;
            match request {
                WheelRequest::Run {
                    wheel_id,
                    slot,
                    generation,
                } => {
                    if generation != self.wheel_generation
                        || self.wheel_active.as_deref() != Some(&wheel_id)
                    {
                        continue;
                    }
                    let target = self.wheel_target;
                    let target_unchanged = target.is_some()
                        && omnitrigger_windows::capture_shortcut_target().ok() == target;
                    let selected = self
                        .active_wheel
                        .as_ref()
                        .and_then(|wheel| wheel.selection(slot));
                    self.hide_wheel(cx);
                    if self.paused || self.recording || !self.ready || !target_unchanged {
                        self.status = "轮盘已取消：前台、监听或录制状态已变化。".into();
                        continue;
                    }
                    if let (Some((id, name, entry)), Some(target)) = (selected, target) {
                        self.execute_wheel_entry(id, name, entry, target, cx);
                    }
                }
                WheelRequest::Configure => {
                    self.hide_wheel(cx);
                    self.page = Page::Wheels;
                    let _ = self
                        .main_window
                        .update(cx, |_, window, _| window.activate_window());
                }
                WheelRequest::Close => {}
            }
        }
        let visible = self.wheel_window.is_some_and(|wheel| {
            wheel
                .update(cx, |view, _, _| view.is_visible())
                .unwrap_or(false)
        });
        let cancelled = visible
            && (self.recording
                || self.paused
                || !self.ready
                || self.wheel_target.is_some_and(|target| {
                    omnitrigger_windows::capture_shortcut_target().ok() != Some(target)
                }));
        if (self.wheel_active.is_some() && !visible) || cancelled {
            self.hide_wheel(cx);
            changed = true;
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displayed_wheel_binding_is_the_first_enabled_match_or_first_disabled_match() {
        let mut bindings = crate::settings::defaults();
        let action = ActionSpec::ShowWheel(WheelActionRef {
            wheel_id: "edit".into(),
        });
        bindings[0].action = action.clone();
        bindings[0].enabled = false;
        bindings[1].action = action.clone();
        bindings[1].enabled = true;
        bindings[1].spec.activation = Activation::Hold {
            button: omnitrigger_core::Button::Key(119),
            duration_ms: 640,
        };
        bindings[2].action = action;
        bindings[2].enabled = true;
        assert_eq!(wheel_binding(&bindings, "edit"), Some(&bindings[1]));
        bindings[1].enabled = false;
        bindings[2].enabled = false;
        assert_eq!(wheel_binding(&bindings, "edit"), Some(&bindings[0]));
        assert!(wheel_binding(&bindings, "another").is_none());
    }

    fn at(ring: u8, sector: u8, sectors: u8) -> Option<WheelPosition> {
        Some(WheelPosition {
            ring,
            sector,
            sectors,
            half_step: false,
        })
    }

    fn wheel(positions: &[Option<WheelPosition>]) -> WheelDefinition {
        let mut wheel = Wheels::default().wheels.remove(0);
        let template = wheel.slots[0].clone();
        wheel.slots = positions
            .iter()
            .enumerate()
            .map(|(index, position)| WheelSlot {
                label: format!("槽位{index}"),
                position: *position,
                ..template.clone()
            })
            .collect();
        wheel
    }

    #[test]
    fn saving_a_wheel_edit_keeps_only_the_matching_running_action() {
        use crate::{
            execution::{CommandAction, CommandExecutor},
            wheel_scenes::TargetContext,
        };
        let command = CommandAction {
            program: std::env::current_exe()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            args: vec!["--list".into(), "--exact".into(), "no_fixture_tests".into()],
            working_directory: None,
        };
        let mut definition = wheel(&[at(0, 0, 8), at(0, 1, 8), at(0, 2, 8)]);
        definition.slots[1].action = ActionSpec::Command(command.clone());
        let before = Wheels {
            wheels: vec![definition],
        };
        let disk = crate::wheels::tests::Scratch::new();
        disk.save(&before).unwrap();
        let mut edited_config = Wheels::load(&disk.bindings()).unwrap();
        let scenes = CompiledScenes::default();
        let edited = &mut edited_config.wheels[0];
        let mut view = scope_view(edited, None, &scenes);
        remove_at(&mut view, (0, 0)).unwrap();
        place_slot(&mut view, 0, 1, 4).unwrap();
        view.slots[0].label = "改名后的命令".into();
        store_view(edited, None, view, &scenes).unwrap();
        let executor = CommandExecutor::new();
        let mut ids = HashMap::new();
        let mut triggers =
            crate::service::TriggerService::new(Vec::new(), crate::service::SettingsFile);
        let mut open = |wheels: &Wheels, ids: &mut HashMap<_, _>| {
            ActiveWheel::for_target(
                &wheels.wheels[0],
                &scenes,
                None,
                TargetContext {
                    process: None,
                    role: omnitrigger_core::WindowRole::Unknown,
                },
                ids,
                &mut triggers,
            )
            .unwrap()
        };
        let initial = open(&before, &mut ids);
        let running = initial.runtime_ids[1];
        executor.submit(running, command.clone()).unwrap();
        disk.save(&edited_config).unwrap();
        let after = Wheels::load(&disk.bindings()).unwrap();
        remap_wheel_runtime_ids(&before, &after, &mut ids);
        let saved = open(&after, &mut ids);
        assert_eq!(saved.runtime_ids, initial.runtime_ids[1..]);
        assert!(executor
            .submit(saved.runtime_ids[0], command.clone())
            .is_err());

        let mut replaced = after.clone();
        let mut next_command = command.clone();
        next_command.args.push("different-fixture".into());
        put_action(
            &mut replaced.wheels[0],
            (1, 4),
            "新命令".into(),
            ActionSpec::Command(next_command.clone()),
        )
        .unwrap();
        disk.save(&replaced).unwrap();
        let replaced = Wheels::load(&disk.bindings()).unwrap();
        remap_wheel_runtime_ids(&after, &replaced, &mut ids);
        let fresh = open(&replaced, &mut ids);
        assert_eq!(fresh.runtime_ids[1], saved.runtime_ids[1]);
        assert!(executor.is_running(running));
        assert!(!executor.is_running(fresh.runtime_ids[0]));
        executor.submit(fresh.runtime_ids[0], next_command).unwrap();
    }

    #[test]
    fn remapping_duplicate_actions_prefers_same_cells_and_unique_names_within_layout() {
        let mut wheel = wheel(&[at(0, 0, 8), at(0, 1, 8), at(0, 2, 8)]);
        for slot in &mut wheel.slots {
            slot.action = ActionSpec::Notification;
        }
        wheel.slots[0].label = "重复".into();
        wheel.slots[2].label = "重复".into();
        let scene_slot = WheelSlot {
            scene: Some("edit".into()),
            ..wheel.slots[0].clone()
        };
        wheel.slots.push(scene_slot);
        wheel
            .app_overrides
            .push(crate::wheels::WheelApplicationOverride {
                process: "Editor.EXE".into(),
                slots: wheel.slots.clone(),
            });
        let before = Wheels {
            wheels: vec![wheel],
        };
        let mut after = before.clone();
        place_slot(&mut after.wheels[0], 0, 0, 1).unwrap();
        let app = &mut after.wheels[0].app_overrides[0];
        app.process = "editor.exe".into();
        for (i, slot) in app.slots.iter_mut().enumerate() {
            slot.position = at(1, i as u8, 8);
        }
        app.slots.swap(0, 1);
        let key = |app, index| WheelSlotKey::new(&before.wheels[0], app, index);
        let mut ids = HashMap::from([
            (key(None, 0), 10),
            (key(Some("editor.exe"), 0), 20),
            (key(Some("editor.exe"), 1), 21),
            (key(Some("editor.exe"), 2), 22),
            (key(Some("editor.exe"), 3), 23),
        ]);
        remap_wheel_runtime_ids(&before, &after, &mut ids);
        assert_eq!(
            ids,
            HashMap::from([
                (key(None, 0), 10),
                (key(Some("editor.exe"), 0), 21),
                (key(Some("editor.exe"), 3), 23),
            ])
        );
    }

    #[test]
    fn running_slots_follow_their_source_across_application_and_scene_changes() {
        use crate::{
            execution::{CommandAction, CommandExecutor},
            wheel_scenes::TargetContext,
        };
        use omnitrigger_core::WindowRole;
        let command = CommandAction {
            program: std::env::current_exe()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            args: vec!["--list".into(), "--exact".into(), "no_fixture_tests".into()],
            working_directory: None,
        };
        let mut definition = wheel(&[at(0, 0, 8), at(0, 1, 8), at(0, 0, 8)]);
        for slot in &mut definition.slots {
            slot.action = ActionSpec::Command(command.clone());
        }
        definition.slots[2].scene = Some("edit".into());
        definition.app_overrides = vec![crate::wheels::WheelApplicationOverride {
            process: "Browser.EXE".into(),
            slots: vec![WheelSlot {
                label: "网页动作".into(),
                ..definition.slots[0].clone()
            }],
        }];
        let scenes = crate::scenes::compile(
            &serde_json::from_value(serde_json::json!({
                "edit":{"process_scope":{"allow":["editor"]}}
            }))
            .unwrap(),
            &crate::app_sets::AppSets::new(),
        )
        .unwrap();
        let mut ids = std::collections::HashMap::new();
        let mut triggers =
            crate::service::TriggerService::new(Vec::new(), crate::service::SettingsFile);
        let mut open = |process| {
            ActiveWheel::for_target(
                &definition,
                &scenes,
                Some(process),
                TargetContext {
                    process: Some(process),
                    role: WindowRole::Application,
                },
                &mut ids,
                &mut triggers,
            )
            .unwrap()
        };
        let global = open("other.exe");
        let browser = open("browser.exe");
        let editor = open("editor.exe");
        let executor = CommandExecutor::new();
        let (run_id, _, running) = global.selection(1).unwrap();
        executor.submit(run_id, command.clone()).unwrap();
        let (editor_id, _, moved) = editor.selection(0).unwrap();
        assert_eq!((editor_id, moved), (run_id, running));
        assert!(executor.is_running(editor_id));
        assert!(executor.submit(editor_id, command.clone()).is_err());
        for (snapshot, index) in [(&global, 0), (&editor, 1), (&browser, 0)] {
            let id = snapshot.selection(index).unwrap().0;
            assert!(!executor.is_running(id));
            executor.submit(id, command.clone()).unwrap();
        }
        assert_eq!(browser.runtime_ids, open("BROWSER.exe").runtime_ids);
        assert_eq!(global.runtime_ids, open("third.exe").runtime_ids);
        assert!(browser.selection(1).is_none());
        for (scope, snapshot, index) in [
            (None, &global, 0),
            (Some(WheelScope::Scene("edit".into())), &editor, 0),
            (Some(WheelScope::Scene("edit".into())), &editor, 1),
            (Some(WheelScope::App("Browser.EXE".into())), &browser, 0),
        ] {
            let view = scope_view(&definition, scope.as_ref(), &scenes);
            let key = editor_slot_key(&definition, scope.as_ref(), &view.slots[index]).unwrap();
            assert_eq!(ids.get(&key), snapshot.runtime_ids.get(index));
        }
    }

    #[test]
    fn reload_keeps_editable_empty_cells_and_clears_directions_removed_by_resize() {
        let view = wheel(&[at(1, 0, 4)]);
        assert_eq!(retained_cell(&view, Some((1, 0))), Some((1, 0)));
        assert_eq!(retained_cell(&view, Some((1, 3))), Some((1, 3)));
        assert_eq!(retained_cell(&view, Some((2, 7))), Some((2, 7)));
        assert_eq!(retained_cell(&view, Some((1, 4))), None);
        assert_eq!(retained_cell(&view, Some((3, 0))), None);
        assert_eq!(retained_cell(&view, None), None);
    }

    #[test]
    fn editor_preview_reads_selected_scope_and_keeps_empty_cells_empty() {
        let mut definition = wheel(&[at(0, 0, 8)]);
        let mut application = definition.slots[0].clone();
        application.label = "专用动作".into();
        application.position = at(1, 3, 8);
        let process = "test.exe".to_owned();
        definition
            .app_overrides
            .push(crate::wheels::WheelApplicationOverride {
                process: process.clone(),
                slots: vec![application],
            });
        let original = definition.clone();
        let view = scope_view(
            &definition,
            Some(&WheelScope::App(process)),
            &CompiledScenes::default(),
        );
        let mut preview = WheelPreview::default();
        assert!(preview.selected_slot(&view).is_none());
        preview.selected = Some((0, 0));
        assert!(preview.selected_slot(&view).is_none());
        preview.selected = Some((1, 3));
        assert_eq!(preview.selected_slot(&view).unwrap().label, "专用动作");
        assert_eq!(definition, original);
    }

    #[test]
    fn editor_preview_demonstrates_only_notifications_and_own_history() {
        let catalog = omnitrigger_actions::PackageCatalog::default();
        assert_eq!(
            preview_action(&ActionSpec::Notification, &catalog),
            PreviewAction::Notification
        );
        let mut call = omnitrigger_actions::ActionCall {
            package_id: "examples.n10-rust".into(),
            action_id: "toggle".into(),
            action_version: 1,
            input: serde_json::json!({}),
        };
        assert_eq!(
            preview_action(&ActionSpec::Invoke(call.clone()), &catalog),
            PreviewAction::BrowseHistory
        );
        call.input = serde_json::json!({"profile":"script-test"});
        assert_eq!(
            preview_action(&ActionSpec::Invoke(call.clone()), &catalog),
            PreviewAction::Describe
        );
        call.input = serde_json::json!({"profile":"other"});
        let external = crate::external_action::ExternalAction {
            provider: "测试".into(),
            command: crate::execution::CommandAction {
                program: "example.exe".into(),
                args: vec![],
                working_directory: None,
            },
            disabled_reason: None,
        };
        for action in [
            ActionSpec::Invoke(call),
            ActionSpec::Command(external.command.clone()),
            ActionSpec::External(external),
            preset_action(&Preset::Keys("Ctrl+W")),
            preset_action(&Preset::Window(WindowOperation::Close)),
        ] {
            assert_eq!(preview_action(&action, &catalog), PreviewAction::Describe);
        }
    }

    #[test]
    fn placing_on_occupied_direction_swaps() {
        let mut wheel = wheel(&[at(0, 0, 8), at(0, 3, 8)]);
        place_slot(&mut wheel, 0, 0, 3).unwrap();
        assert_eq!(wheel.slots[0].position, at(0, 3, 8));
        assert_eq!(wheel.slots[1].position, at(0, 0, 8));
        wheel.validate().unwrap();
    }

    #[test]
    fn swapping_across_rings_keeps_each_ring_geometry() {
        let mut wheel = wheel(&[at(0, 1, 8), at(1, 5, 16)]);
        place_slot(&mut wheel, 0, 1, 5).unwrap();
        assert_eq!(wheel.slots[0].position, at(1, 5, 16));
        assert_eq!(wheel.slots[1].position, at(0, 1, 8));
        wheel.validate().unwrap();
    }

    #[test]
    fn putting_on_empty_cell_uses_ring_geometry() {
        let mut view = wheel(&[at(0, 0, 6), at(1, 2, 12)]);
        let action = view.slots[0].action.clone();
        put_action(&mut view, (1, 7), "新".into(), action.clone()).unwrap();
        assert_eq!(view.slots[2].position, at(1, 7, 12));
        put_action(&mut view, (2, 3), "外".into(), action.clone()).unwrap();
        assert_eq!(view.slots[3].position, at(2, 3, 8));
        assert!(put_action(&mut view, (0, 6), "越界".into(), action.clone()).is_err());
        put_action(&mut view, (0, 0), "替换".into(), action).unwrap();
        assert_eq!(
            (view.slots.len(), view.slots[0].label.as_str()),
            (4, "替换")
        );
        view.validate().unwrap();
    }

    #[test]
    fn removing_keeps_other_positions_and_last_slot() {
        let mut view = wheel(&[at(0, 0, 8), at(0, 4, 8)]);
        remove_at(&mut view, (0, 0)).unwrap();
        assert_eq!(view.slots[0].position, at(0, 4, 8));
        assert!(remove_at(&mut view, (0, 4)).is_err());
        assert!(remove_at(&mut view, (0, 1)).is_err());
    }

    #[test]
    fn scope_view_edits_app_layout_and_converts_even_layout() {
        let mut definition = wheel(&[None, None]);
        definition
            .app_overrides
            .push(crate::wheels::WheelApplicationOverride {
                process: "code.exe".into(),
                slots: wheel(&[at(1, 3, 4)]).slots,
            });
        let code = WheelScope::App("code.exe".into());
        let mut view = scope_view(&definition, Some(&code), &none());
        assert!(view.app_overrides.is_empty());
        let action = view.slots[0].action.clone();
        put_action(&mut view, (1, 0), "应用".into(), action).unwrap();
        store_view(&mut definition, Some(&code), view, &none()).unwrap();
        assert_eq!(definition.app_overrides[0].slots.len(), 2);
        assert_eq!(definition.slots[0].position, None);
        let general = scope_view(&definition, None, &none());
        assert_eq!(general.slots[1].position, at(0, 1, 2));
        store_view(&mut definition, None, general, &none()).unwrap();
        definition.validate().unwrap();
    }

    fn none() -> CompiledScenes {
        CompiledScenes::default()
    }

    fn labels(view: &WheelDefinition) -> Vec<(&str, Option<&str>)> {
        view.slots
            .iter()
            .map(|slot| (slot.label.as_str(), slot.scene.as_deref()))
            .collect()
    }

    #[test]
    fn scene_view_copies_shared_slots_on_write_and_keeps_shared_layout_intact() {
        let mut definition = wheel(&[at(0, 0, 8), at(0, 1, 8)]);
        let chrome = WheelScope::Scene("chrome".into());
        // 未改动时写回不产生专属槽位。
        let view = scope_view(&definition, Some(&chrome), &none());
        store_view(&mut definition, Some(&chrome), view, &none()).unwrap();
        assert_eq!(definition.slots.len(), 2);
        // 替换通用格子与在空格新增都成为场景专属槽位，通用槽位不变。
        let mut view = scope_view(&definition, Some(&chrome), &none());
        let action = view.slots[0].action.clone();
        put_action(&mut view, (0, 1), "关闭网页".into(), action.clone()).unwrap();
        put_action(&mut view, (0, 4), "新标签".into(), action).unwrap();
        store_view(&mut definition, Some(&chrome), view, &none()).unwrap();
        definition.validate().unwrap();
        assert_eq!(
            labels(&definition),
            [
                ("槽位0", None),
                ("槽位1", None),
                ("关闭网页", Some("chrome")),
                ("新标签", Some("chrome"))
            ]
        );
        assert_eq!(
            labels(&scope_view(&definition, Some(&chrome), &none())),
            [
                ("槽位0", None),
                ("关闭网页", Some("chrome")),
                ("新标签", Some("chrome"))
            ]
        );
        assert_eq!(
            labels(&scope_view(&definition, None, &none())),
            [("槽位0", None), ("槽位1", None)]
        );
        // 场景中不能删除、移走通用动作或调整格数；删除专属槽位即恢复通用动作。
        for edit in [
            (|view: &mut WheelDefinition| remove_at(view, (0, 0))) as fn(&mut _) -> _,
            |view| place_slot(view, 0, 0, 6),
            |view| set_ring_layout(view, 0, 12, false),
        ] {
            let mut view = scope_view(&definition, Some(&chrome), &none());
            edit(&mut view).unwrap();
            let before = definition.clone();
            assert!(store_view(&mut definition, Some(&chrome), view, &none()).is_err());
            assert_eq!(definition, before);
        }
        let mut view = scope_view(&definition, Some(&chrome), &none());
        remove_at(&mut view, (0, 1)).unwrap();
        store_view(&mut definition, Some(&chrome), view, &none()).unwrap();
        assert_eq!(
            labels(&scope_view(&definition, Some(&chrome), &none())),
            [("槽位0", None), ("槽位1", None), ("新标签", Some("chrome"))]
        );
    }

    #[test]
    fn scene_view_shows_slots_of_scenes_it_implies_and_copies_them_on_write() {
        let scenes = crate::scenes::compile(
            &serde_json::from_value(serde_json::json!({
                "chrome": {"process_scope": {"allow": ["chrome"]}},
                "not-desktop": {"process_scope": {"identity": "shell_context", "deny": ["desktop"]}},
                "not-chrome": {"process_scope": {"deny": ["chrome"]}}
            }))
            .unwrap(),
            &crate::app_sets::AppSets::new(),
        )
        .unwrap();
        let mut definition = wheel(&[at(0, 0, 8)]);
        for (sector, scene) in [(2, "not-desktop"), (3, "not-chrome")] {
            definition.slots.push(WheelSlot {
                label: scene.into(),
                scene: Some(scene.into()),
                ..wheel(&[at(0, sector, 8)]).slots.remove(0)
            });
        }
        let chrome = WheelScope::Scene("chrome".into());
        // chrome 窗口必然不是桌面，因此沿用“not-desktop”的槽位；“not-chrome”不显示。
        assert_eq!(
            labels(&scope_view(&definition, Some(&chrome), &scenes)),
            [("槽位0", None), ("not-desktop", Some("not-desktop"))]
        );
        let mut view = scope_view(&definition, Some(&chrome), &scenes);
        let unchanged = definition.clone();
        store_view(&mut definition, Some(&chrome), view.clone(), &scenes).unwrap();
        assert_eq!(definition, unchanged);
        view.slots[1].label = "窗口调整".into();
        store_view(&mut definition, Some(&chrome), view, &scenes).unwrap();
        definition.validate().unwrap();
        assert_eq!(
            labels(&definition),
            [
                ("槽位0", None),
                ("not-desktop", Some("not-desktop")),
                ("not-chrome", Some("not-chrome")),
                ("窗口调整", Some("chrome"))
            ]
        );
        let mut view = scope_view(
            &definition,
            Some(&WheelScope::Scene("not-desktop".into())),
            &scenes,
        );
        assert_eq!(
            labels(&view),
            [("槽位0", None), ("not-desktop", Some("not-desktop"))]
        );
        remove_at(&mut view, (0, 2)).unwrap();
        store_view(
            &mut definition,
            Some(&WheelScope::Scene("not-desktop".into())),
            view,
            &scenes,
        )
        .unwrap();
        assert_eq!(definition.slots.len(), 3);
    }

    #[test]
    fn shared_edits_keep_scene_slots_and_carry_ring_geometry_to_them() {
        let mut definition = wheel(&[at(0, 0, 8), at(0, 1, 8)]);
        definition.slots.push(WheelSlot {
            scene: Some("chrome".into()),
            ..wheel(&[at(0, 5, 8)]).slots.remove(0)
        });
        let mut view = scope_view(&definition, None, &none());
        set_ring_layout(&mut view, 0, 12, true).unwrap();
        store_view(&mut definition, None, view, &none()).unwrap();
        definition.validate().unwrap();
        assert_eq!(definition.slots.len(), 3);
        assert_eq!(
            definition.slots[2].position,
            Some(WheelPosition {
                ring: 0,
                sector: 5,
                sectors: 12,
                half_step: true,
            })
        );
        let mut view = scope_view(&definition, None, &none());
        set_ring_layout(&mut view, 0, 4, false).unwrap();
        let before = definition.clone();
        assert!(store_view(&mut definition, None, view, &none())
            .unwrap_err()
            .contains("场景 chrome"));
        assert_eq!(definition, before);
    }

    #[test]
    fn presets_parse() {
        for (_, preset) in &WHEEL_PRESETS {
            preset_action(preset);
        }
    }

    #[test]
    fn ring_layout_applies_to_whole_ring_and_refuses_to_drop_slots() {
        let mut wheel = wheel(&[at(0, 6, 8), at(0, 1, 8), at(1, 0, 16)]);
        assert!(set_ring_layout(&mut wheel, 0, 6, false).is_err());
        set_ring_layout(&mut wheel, 0, 12, true).unwrap();
        for slot in &wheel.slots[..2] {
            let position = slot.position.unwrap();
            assert_eq!((position.sectors, position.half_step), (12, true));
        }
        assert_eq!(wheel.slots[2].position, at(1, 0, 16));
        wheel.validate().unwrap();
    }

    #[test]
    fn converting_even_layout_keeps_directions() {
        let mut wheel = wheel(&[None, None, None]);
        convert_to_positions(&mut wheel);
        let positions: Vec<_> = wheel.slots.iter().map(|slot| slot.position).collect();
        assert_eq!(positions, [at(0, 0, 3), at(0, 1, 3), at(0, 2, 3)]);
        wheel.validate().unwrap();
        assert!(place_slot(&mut wheel, 0, 0, 3).is_err());
    }
}
