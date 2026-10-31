//! 轮盘管理与执行协调；浮窗只提交选择，不在输入 Hook 内执行动作。
use crate::{
    radial_wheel::RingGeometry,
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
use std::time::Duration;

pub(super) struct DragWheelSession {
    session: u64,
    binding_id: u64,
    definition: WheelDefinition,
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

/// 返回占用该方位的槽位序号。
pub(super) fn slot_at(wheel: &WheelDefinition, ring: u8, sector: u8) -> Option<usize> {
    wheel.slots.iter().position(|slot| {
        slot.position
            .is_some_and(|position| position.ring == ring && position.sector == sector)
    })
}

/// 编辑时取出一组槽位：通用布局或某个应用的专用布局；均分布局转为方位布局。
pub(super) fn scope_view(wheel: &WheelDefinition, scope: Option<&str>) -> WheelDefinition {
    let mut view = wheel.clone();
    if let Some(application) = scope.and_then(|process| {
        wheel
            .app_overrides
            .iter()
            .find(|application| application.process == process)
    }) {
        view.slots = application.slots.clone();
    }
    view.app_overrides.clear();
    convert_to_positions(&mut view);
    view
}

/// 把编辑后的槽位写回对应的布局。
pub(super) fn store_view(wheel: &mut WheelDefinition, scope: Option<&str>, view: WheelDefinition) {
    match scope.and_then(|process| {
        wheel
            .app_overrides
            .iter_mut()
            .find(|application| application.process == process)
    }) {
        Some(application) => application.slots = view.slots,
        None => wheel.slots = view.slots,
    }
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

    fn commit_wheels(&mut self, proposed: Wheels, cx: &mut Context<Self>) -> bool {
        if let Some(error) = &self.wheels_error {
            self.error = Some(format!("原轮盘配置不可用，请先修复文件：{error}"));
            cx.notify();
            return false;
        }
        // 文件可能已在外部编辑器中修改；不同于已载入内容时拒绝保存，避免覆盖这些修改。
        if Wheels::load(&crate::settings::path()).is_ok_and(|disk| disk != self.wheels) {
            self.error = Some(
                "wheels.json 已在外部修改，请先点击“重新载入”，再在页面中编辑，以免覆盖文件中的修改。"
                    .into(),
            );
            cx.notify();
            return false;
        }
        match proposed.save(&crate::settings::path()) {
            Ok(()) => {
                self.apply_wheels(proposed, cx);
                self.status = "轮盘已保存。".into();
                true
            }
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                false
            }
        }
    }

    fn apply_wheels(&mut self, wheels: Wheels, cx: &mut Context<Self>) {
        self.hide_wheel(cx);
        self.wheels = wheels;
        self.wheels_error = None;
        self.error = None;
        let definitions = &self.wheels.wheels;
        self.wheel_runtime_ids.retain(|(id, index), runtime_id| {
            definitions
                .iter()
                .any(|wheel| &wheel.id == id && *index < wheel.slots.len())
                || self.executor.is_running(*runtime_id)
                || self.shortcut_executor.is_running(*runtime_id)
                || self.system_executor.is_running(*runtime_id)
                || self.local_executor.is_running(*runtime_id)
                || self.action_service.is_running(*runtime_id)
        });
        self.sync_bindings();
        cx.notify();
    }

    /// 重新读取 wheels.json；读取或校验失败时保留当前内容并显示原因。
    pub(super) fn reload_wheels(&mut self, cx: &mut Context<Self>) {
        match Wheels::load(&crate::settings::path()) {
            Ok(wheels) => {
                let (cell, scope) = (self.wheel_cell, self.wheel_scope.clone());
                self.apply_wheels(wheels, cx);
                let id = self
                    .wheel_selected
                    .clone()
                    .filter(|id| self.wheels.wheels.iter().any(|wheel| &wheel.id == id))
                    .or_else(|| self.wheels.wheels.first().map(|wheel| wheel.id.clone()));
                match id {
                    Some(id) => self.select_wheel(id, cx),
                    None => self.wheel_selected = None,
                }
                self.wheel_scope = scope;
                self.wheel_cell = cell;
                self.load_wheel_cell(cx);
                self.status = "已重新载入轮盘配置。".into();
            }
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
        if !path.exists() && self.wheels_error.is_none() {
            if let Err(error) = self.wheels.save(&crate::settings::path()) {
                self.error = Some(error);
                cx.notify();
                return;
            }
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

    /// 当前编辑的应用范围；所选应用已不在专用布局中时回到通用布局。
    fn wheel_scope_for(&self, wheel: &WheelDefinition) -> Option<String> {
        self.wheel_scope.clone().filter(|process| {
            wheel
                .app_overrides
                .iter()
                .any(|application| &application.process == process)
        })
    }

    /// 选中轮盘在当前应用范围下的槽位，均分布局已转为方位布局。
    pub(super) fn wheel_view(&self) -> Option<WheelDefinition> {
        let wheel = self.wheels.wheels.get(self.selected_wheel_index()?)?;
        Some(scope_view(wheel, self.wheel_scope_for(wheel).as_deref()))
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
        let mut view = scope_view(wheel, scope.as_deref());
        if let Err(error) = edit(&mut view) {
            self.error = Some(error);
            cx.notify();
            return false;
        }
        store_view(wheel, scope.as_deref(), view);
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

    /// 名称在离开格子前自动保存；内容未变或为空时不写文件。
    fn flush_wheel_label(&mut self, cx: &mut Context<Self>) {
        let Some((view, (ring, sector))) = self.wheel_view().zip(self.wheel_cell) else {
            return;
        };
        let Some(slot) = slot_at(&view, ring, sector) else {
            return;
        };
        let label = self.wheel_slot_label.read(cx).text().trim().to_string();
        if label.is_empty() || label == view.slots[slot].label {
            return;
        }
        self.edit_wheel_view("名称已保存。", cx, |view| {
            view.slots[slot].label = label;
            Ok(())
        });
    }

    /// 点格子：移动中则把动作移到这里（有动作就互换），否则选中或取消选中。
    pub(super) fn select_wheel_cell(&mut self, cell: (u8, u8), cx: &mut Context<Self>) {
        if self.wheel_moving {
            self.wheel_moving = false;
            if let Some(from) = self.wheel_cell.filter(|from| *from != cell) {
                if self.edit_wheel_view("已移动。", cx, |view| {
                    let slot = slot_at(view, from.0, from.1).ok_or("要移动的动作已不存在。")?;
                    place_slot(view, slot, cell.0, cell.1)
                }) {
                    self.wheel_cell = Some(cell);
                    self.load_wheel_cell(cx);
                }
            }
            cx.notify();
            return;
        }
        self.flush_wheel_label(cx);
        self.wheel_cell = (self.wheel_cell != Some(cell)).then_some(cell);
        self.wheel_replacing = false;
        self.wheel_source_binding = None;
        self.load_wheel_cell(cx);
        cx.notify();
    }

    pub(super) fn deselect_wheel_cell(&mut self, cx: &mut Context<Self>) {
        self.flush_wheel_label(cx);
        self.wheel_cell = None;
        self.wheel_moving = false;
        self.wheel_replacing = false;
        self.load_wheel_cell(cx);
        cx.notify();
    }

    pub(super) fn set_wheel_moving(&mut self, moving: bool, cx: &mut Context<Self>) {
        self.flush_wheel_label(cx);
        self.wheel_moving = moving;
        cx.notify();
    }

    pub(super) fn set_wheel_replacing(&mut self, replacing: bool, cx: &mut Context<Self>) {
        self.flush_wheel_label(cx);
        self.wheel_replacing = replacing;
        self.wheel_source_binding = None;
        cx.notify();
    }

    pub(super) fn set_wheel_scope(&mut self, scope: Option<String>, cx: &mut Context<Self>) {
        self.flush_wheel_label(cx);
        self.wheel_scope = scope;
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
        self.flush_wheel_label(cx);
        self.edit_wheel_view("本地动作已保存。", cx, |view| {
            let slot = slot_at(view, ring, sector).ok_or("这个格子是空的。")?;
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
        self.flush_wheel_label(cx);
        self.select_wheel(id, cx);
    }

    pub(super) fn select_wheel(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(wheel) = self.wheels.wheels.iter().find(|wheel| wheel.id == id) else {
            return;
        };
        let name = wheel.name.clone();
        self.smooth_scroll.cancel("wheel-editor-content");
        self.smooth_scroll.cancel("wheel-source-list");
        self.wheel_selected = Some(id);
        self.wheel_cell = None;
        self.wheel_moving = false;
        self.wheel_replacing = false;
        self.wheel_scope = None;
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
        if let Some(existing) = self.triggers.bindings().iter().find(|binding|
            matches!(&binding.action, ActionSpec::ShowWheel(reference) if reference.wheel_id == wheel.id)
                && matches!(binding.spec.activation, Activation::Drag { .. })).map(|binding| binding.spec.id) {
            self.clear_pending();
            self.selected = Some(existing);
            self.load_selected_message(cx);
            self.page = Page::Triggers;
            cx.notify();
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
        if let Some(id) = self.wheel_selected.clone() {
            self.open_wheel(&id, cx);
        }
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
        self.wheel_active_definition = None;
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

    fn wheel_slot_states(&self, definition: &WheelDefinition) -> Vec<Option<String>> {
        definition
            .slots
            .iter()
            .enumerate()
            .map(|(index, slot)| {
                if self.paused || !self.ready {
                    return Some("监听尚未就绪或已暂停。".into());
                }
                self.action_unavailable_reason(&slot.action).or_else(|| {
                    self.wheel_runtime_ids
                        .get(&(definition.id.clone(), index))
                        .filter(|id| self.action_busy(**id, &slot.action))
                        .map(|_| "此动作仍在执行，请稍后重试。".into())
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
        let definition = definition.resolved_for_process(process.as_deref());
        let wheel_size = crate::radial_wheel::wheel_size(&definition);
        let window = match self.prepare_wheel(cx) {
            Ok(window) => window,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        for index in 0..definition.slots.len() {
            if !self
                .wheel_runtime_ids
                .contains_key(&(id.to_string(), index))
            {
                let Some(runtime_id) = self.draft_id(None) else {
                    cx.notify();
                    return;
                };
                self.wheel_runtime_ids
                    .insert((id.to_string(), index), runtime_id);
            }
        }
        let states = self.wheel_slot_states(&definition);
        let generation = self.wheel_generation;
        let wheel_id = id.to_string();
        self.wheel_active_definition = Some(definition.clone());
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
            if let Some(definition) = &self.wheel_active_definition {
                let states = self.wheel_slot_states(definition);
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
                let Some(definition) = self.wheel_active_definition.clone() else {
                    return;
                };
                self.drag_wheel_session = Some(DragWheelSession {
                    session: event.session,
                    binding_id: event.id,
                    definition,
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
                    &drag.definition,
                    drag.cancel_radius_px,
                    drag.scale,
                );
                if let Some(index) = selected {
                    if let (Some(entry), Some(id)) = (
                        drag.definition.slots.get(index).cloned(),
                        self.wheel_runtime_ids
                            .get(&(drag.definition.id.clone(), index))
                            .copied(),
                    ) {
                        self.execute_wheel_entry(id, drag.definition.name, entry, drag.target, cx);
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
                    let selected = self.wheel_active_definition.as_ref().and_then(|wheel| {
                        wheel
                            .slots
                            .get(slot)
                            .cloned()
                            .map(|entry| (wheel.name.clone(), entry))
                    });
                    let runtime_id = self.wheel_runtime_ids.get(&(wheel_id, slot)).copied();
                    self.hide_wheel(cx);
                    if self.paused || self.recording || !self.ready || !target_unchanged {
                        self.status = "轮盘已取消：前台、监听或录制状态已变化。".into();
                        continue;
                    }
                    if let (Some((name, entry)), Some(id), Some(target)) =
                        (selected, runtime_id, target)
                    {
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
        let mut view = scope_view(&definition, Some("code.exe"));
        assert!(view.app_overrides.is_empty());
        let action = view.slots[0].action.clone();
        put_action(&mut view, (1, 0), "应用".into(), action).unwrap();
        store_view(&mut definition, Some("code.exe"), view);
        assert_eq!(definition.app_overrides[0].slots.len(), 2);
        assert_eq!(definition.slots[0].position, None);
        let general = scope_view(&definition, None);
        assert_eq!(general.slots[1].position, at(0, 1, 2));
        store_view(&mut definition, None, general);
        definition.validate().unwrap();
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
