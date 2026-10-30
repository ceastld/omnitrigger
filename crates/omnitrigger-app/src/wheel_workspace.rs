//! 轮盘管理与执行协调；浮窗只提交选择，不在输入 Hook 内执行动作。
use crate::{
    wheels::{WheelDefinition, WheelPosition, WheelSlot},
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

/// 显式布局保留空方向；新增动作填入已有圈中的首个空位。
pub(super) fn next_slot_position(wheel: &WheelDefinition) -> Option<crate::wheels::WheelPosition> {
    let _ = wheel.slots.first()?.position?;
    for ring in 0..=2 {
        let Some(template) = wheel
            .slots
            .iter()
            .filter_map(|slot| slot.position)
            .find(|position| position.ring == ring)
        else {
            continue;
        };
        for sector in 0..template.sectors {
            if !wheel
                .slots
                .iter()
                .filter_map(|slot| slot.position)
                .any(|position| position.ring == ring && position.sector == sector)
            {
                return Some(crate::wheels::WheelPosition { sector, ..template });
            }
        }
    }
    None
}

pub(super) const RING_NAMES: [&str; 3] = ["内圈", "中圈", "外圈"];
/// 尚无动作的圈首次放入槽位时使用的方向数。
const DEFAULT_SECTORS: u8 = 8;

/// 圈的方向数与半格偏移由该圈已有槽位决定；空圈返回 `None`。
pub(super) fn ring_geometry(wheel: &WheelDefinition, ring: u8) -> Option<(u8, bool)> {
    wheel
        .slots
        .iter()
        .filter_map(|slot| slot.position)
        .find(|position| position.ring == ring)
        .map(|position| (position.sectors, position.half_step))
}

/// 返回占用该方位的槽位序号。
pub(super) fn slot_at(wheel: &WheelDefinition, ring: u8, sector: u8) -> Option<usize> {
    wheel.slots.iter().position(|slot| {
        slot.position
            .is_some_and(|position| position.ring == ring && position.sector == sector)
    })
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
        .ok_or("只有方位布局可以调整方向，请先改用方位布局。")?;
    let name = RING_NAMES.get(usize::from(ring)).ok_or("圈号应为 0–2。")?;
    let (sectors, half_step) = ring_geometry(wheel, ring).unwrap_or((DEFAULT_SECTORS, false));
    if sector >= sectors {
        return Err(format!("{name}只有 {sectors} 个方向。"));
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

/// 切换圈时优先保留原方向编号，被占用或超出范围时取该圈第一个空方向。
pub(super) fn move_slot_to_ring(
    wheel: &mut WheelDefinition,
    slot: usize,
    ring: u8,
) -> Result<(), String> {
    let current = wheel
        .slots
        .get(slot)
        .and_then(|entry| entry.position)
        .ok_or("只有方位布局可以调整方向，请先改用方位布局。")?;
    if current.ring == ring {
        return Ok(());
    }
    let name = RING_NAMES.get(usize::from(ring)).ok_or("圈号应为 0–2。")?;
    let sectors = ring_geometry(wheel, ring).map_or(DEFAULT_SECTORS, |(sectors, _)| sectors);
    let sector = std::iter::once(current.sector)
        .chain(0..sectors)
        .find(|sector| *sector < sectors && slot_at(wheel, ring, *sector).is_none())
        .ok_or(format!(
            "{name}已没有空方向，可增加方向数或直接点选方向互换。"
        ))?;
    place_slot(wheel, slot, ring, sector)
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
        return Err("每圈方向数应为 1–16。".into());
    }
    if let Some(position) = wheel
        .slots
        .iter()
        .filter_map(|slot| slot.position)
        .find(|position| position.ring == ring && position.sector >= sectors)
    {
        return Err(format!(
            "{name}方向 {} 已有动作，先把它移到其他方向再减少方向数。",
            position.sector + 1
        ));
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

pub(super) fn wheel_has_room(wheel: &WheelDefinition) -> bool {
    if wheel
        .slots
        .first()
        .is_some_and(|slot| slot.position.is_some())
    {
        next_slot_position(wheel).is_some()
    } else {
        wheel.slots.len() < 8
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
                let slot = self.wheel_slot_selected;
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
                if let Some(slot) = slot {
                    self.select_wheel_slot(slot, cx);
                }
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

    fn edit_selected_wheel(
        &mut self,
        cx: &mut Context<Self>,
        edit: impl FnOnce(&mut WheelDefinition, Option<usize>) -> Result<(), String>,
    ) {
        let Some(index) = self.selected_wheel_index() else {
            return;
        };
        let slot = self.wheel_slot_selected;
        let mut proposed = self.wheels.clone();
        if let Err(error) = edit(&mut proposed.wheels[index], slot) {
            self.error = Some(error);
            cx.notify();
            return;
        }
        if self.commit_wheels(proposed, cx) {
            if let Some(slot) = slot {
                self.select_wheel_slot(slot, cx);
            }
        }
    }

    pub(super) fn place_wheel_slot(&mut self, ring: u8, sector: u8, cx: &mut Context<Self>) {
        self.edit_selected_wheel(cx, |wheel, slot| {
            place_slot(wheel, slot.ok_or("请先选择槽位。")?, ring, sector)
        });
    }

    pub(super) fn move_wheel_slot_ring(&mut self, ring: u8, cx: &mut Context<Self>) {
        self.edit_selected_wheel(cx, |wheel, slot| {
            move_slot_to_ring(wheel, slot.ok_or("请先选择槽位。")?, ring)
        });
    }

    pub(super) fn set_wheel_ring_layout(
        &mut self,
        ring: u8,
        sectors: u8,
        half_step: bool,
        cx: &mut Context<Self>,
    ) {
        self.edit_selected_wheel(cx, |wheel, _| {
            set_ring_layout(wheel, ring, sectors, half_step)
        });
    }

    pub(super) fn convert_wheel_to_positions(&mut self, cx: &mut Context<Self>) {
        self.edit_selected_wheel(cx, |wheel, _| {
            convert_to_positions(wheel);
            Ok(())
        });
    }

    pub(super) fn select_wheel(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(wheel) = self.wheels.wheels.iter().find(|wheel| wheel.id == id) else {
            return;
        };
        let name = wheel.name.clone();
        self.smooth_scroll.cancel("wheel-editor-content");
        self.smooth_scroll.cancel("wheel-source-list");
        self.wheel_selected = Some(id);
        self.wheel_slot_selected = None;
        self.wheel_source_binding = None;
        self.wheel_name
            .update(cx, |input, cx| input.set_text(name, cx));
        self.wheel_slot_label
            .update(cx, |input, cx| input.set_text("", cx));
        self.wheel_slot_shortcut
            .update(cx, |input, cx| input.set_text("Ctrl+C", cx));
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

    pub(super) fn select_wheel_slot(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(wheel_index) = self.selected_wheel_index() else {
            return;
        };
        let Some(slot) = self.wheels.wheels[wheel_index].slots.get(index).cloned() else {
            return;
        };
        self.smooth_scroll.cancel("wheel-editor-content");
        self.wheel_slot_selected = Some(index);
        self.wheel_slot_label
            .update(cx, |input, cx| input.set_text(slot.label, cx));
        if let ActionSpec::Local(action) = &slot.action {
            self.wheel_local_editor.load(action, cx);
        }
        let shortcut = match slot.action {
            ActionSpec::SendShortcut(shortcut) => shortcut.display(),
            _ => String::new(),
        };
        self.wheel_slot_shortcut
            .update(cx, |input, cx| input.set_text(shortcut, cx));
        cx.notify();
    }

    pub(super) fn add_wheel_shortcut(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.selected_wheel_index() else {
            return;
        };
        let shortcut = match ShortcutAction::parse(&self.wheel_slot_shortcut.read(cx).text()) {
            Ok(shortcut) => shortcut,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        let label = self.wheel_slot_label.read(cx).text().trim().to_string();
        let label = if label.is_empty() {
            "新快捷键".into()
        } else {
            label
        };
        let mut proposed = self.wheels.clone();
        let slot_index = proposed.wheels[index].slots.len();
        if !wheel_has_room(&proposed.wheels[index]) {
            self.error = Some("当前布局没有空余方向。".into());
            cx.notify();
            return;
        }
        let position = next_slot_position(&proposed.wheels[index]);
        proposed.wheels[index].slots.push(WheelSlot {
            label,
            action: ActionSpec::SendShortcut(shortcut),
            position,
        });
        if self.commit_wheels(proposed, cx) {
            self.select_wheel_slot(slot_index, cx);
        }
    }

    pub(super) fn add_wheel_local_action(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.selected_wheel_index() else {
            return;
        };
        if !wheel_has_room(&self.wheels.wheels[index]) {
            self.error = Some("当前布局没有空余方向。".into());
            cx.notify();
            return;
        }
        let mut proposed = self.wheels.clone();
        let position = next_slot_position(&proposed.wheels[index]);
        let slot_index = proposed.wheels[index].slots.len();
        proposed.wheels[index].slots.push(WheelSlot {
            label: "本地动作".into(),
            position,
            action: ActionSpec::Local(crate::local_editor::initial_action()),
        });
        if self.commit_wheels(proposed, cx) {
            self.select_wheel_slot(slot_index, cx);
        }
    }

    pub(super) fn save_wheel_slot(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.selected_wheel_index() else {
            return;
        };
        let Some(slot_index) = self.wheel_slot_selected else {
            return;
        };
        let mut proposed = self.wheels.clone();
        let Some(slot) = proposed.wheels[index].slots.get_mut(slot_index) else {
            return;
        };
        slot.label = self.wheel_slot_label.read(cx).text().trim().into();
        if matches!(slot.action, ActionSpec::Local(_)) {
            match self.wheel_local_editor.snapshot(cx) {
                Ok(action) => slot.action = ActionSpec::Local(action),
                Err(error) => {
                    self.error = Some(error);
                    cx.notify();
                    return;
                }
            }
        }
        if matches!(slot.action, ActionSpec::SendShortcut(_)) {
            match ShortcutAction::parse(&self.wheel_slot_shortcut.read(cx).text()) {
                Ok(shortcut) => slot.action = ActionSpec::SendShortcut(shortcut),
                Err(error) => {
                    self.error = Some(error);
                    cx.notify();
                    return;
                }
            }
        }
        self.commit_wheels(proposed, cx);
    }

    pub(super) fn move_wheel_slot(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(index) = self.selected_wheel_index() else {
            return;
        };
        let Some(slot_index) = self.wheel_slot_selected else {
            return;
        };
        let Some(target) = slot_index.checked_add_signed(delta) else {
            return;
        };
        if target >= self.wheels.wheels[index].slots.len() {
            return;
        }
        let mut proposed = self.wheels.clone();
        let previous_position = proposed.wheels[index].slots[slot_index].position;
        proposed.wheels[index].slots[slot_index].position =
            proposed.wheels[index].slots[target].position;
        proposed.wheels[index].slots[target].position = previous_position;
        proposed.wheels[index].slots.swap(slot_index, target);
        if self.commit_wheels(proposed, cx) {
            self.select_wheel_slot(target, cx);
        }
    }

    pub(super) fn remove_wheel_slot(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.selected_wheel_index() else {
            return;
        };
        let Some(slot_index) = self.wheel_slot_selected else {
            return;
        };
        if self.wheels.wheels[index].slots.len() <= 1 {
            self.error = Some("轮盘至少保留一个动作。".into());
            cx.notify();
            return;
        }
        if slot_index >= self.wheels.wheels[index].slots.len() {
            return;
        }
        let mut proposed = self.wheels.clone();
        proposed.wheels[index].slots.remove(slot_index);
        if self.commit_wheels(proposed, cx) {
            self.select_wheel_slot(
                slot_index.min(self.wheels.wheels[index].slots.len() - 1),
                cx,
            );
        }
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
            self.wheel_slot_selected = None;
            if let Some(wheel) = self.wheels.wheels.first() {
                self.select_wheel(wheel.id.clone(), cx);
            }
        }
    }

    pub(super) fn add_wheel_binding(&mut self, binding_id: u64, cx: &mut Context<Self>) {
        let Some(index) = self.selected_wheel_index() else {
            return;
        };
        let Some(binding) = self.binding(binding_id).cloned() else {
            return;
        };
        if matches!(
            binding.action,
            ActionSpec::ShowPanel | ActionSpec::ShowWheel(_)
        ) {
            return;
        }
        let mut proposed = self.wheels.clone();
        let slot_index = proposed.wheels[index].slots.len();
        if !wheel_has_room(&proposed.wheels[index]) {
            self.error = Some("当前布局没有空余方向。".into());
            cx.notify();
            return;
        }
        let position = next_slot_position(&proposed.wheels[index]);
        proposed.wheels[index].slots.push(WheelSlot {
            label: binding.message.chars().take(30).collect(),
            action: binding.action,
            position,
        });
        if self.commit_wheels(proposed, cx) {
            self.wheel_source_binding = Some(binding_id);
            self.select_wheel_slot(slot_index, cx);
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
    fn moving_to_empty_ring_uses_default_sectors_and_keeps_direction() {
        let mut wheel = wheel(&[at(0, 5, 8), at(0, 0, 8)]);
        move_slot_to_ring(&mut wheel, 0, 2).unwrap();
        assert_eq!(wheel.slots[0].position, at(2, 5, 8));
        wheel.validate().unwrap();
    }

    #[test]
    fn moving_to_ring_falls_back_to_first_free_direction() {
        let mut wheel = wheel(&[at(0, 1, 8), at(1, 0, 2), at(1, 1, 2)]);
        assert!(move_slot_to_ring(&mut wheel, 0, 1).is_err());
        set_ring_layout(&mut wheel, 1, 4, false).unwrap();
        move_slot_to_ring(&mut wheel, 0, 1).unwrap();
        assert_eq!(wheel.slots[0].position, at(1, 2, 4));
        wheel.validate().unwrap();
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
