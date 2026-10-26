//! 轮盘管理与执行协调；浮窗只提交选择，不在输入 Hook 内执行动作。
use crate::{
    wheels::{WheelDefinition, WheelSlot},
    ActionSpec, Activation, BackendCommand, Bounds, Button, Context, HitSource, Page, RadialWheel,
    RawWindowHandle, ShortcutAction, WheelAccent, WheelActionRef, WheelRequest, Wheels,
    WindowBounds, WindowHandle, WindowKind, WindowOptions, Workspace, WHEEL_SIZE,
};
use gpui::{prelude::*, px, size};
use raw_window_handle::HasWindowHandle;

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
        match proposed.save(&crate::settings::path()) {
            Ok(()) => {
                self.hide_wheel(cx);
                self.wheels = proposed;
                self.error = None;
                self.status = "轮盘已保存。".into();
                let definitions = &self.wheels.wheels;
                self.wheel_runtime_ids.retain(|(id, index), runtime_id| {
                    definitions
                        .iter()
                        .any(|wheel| &wheel.id == id && *index < wheel.slots.len())
                        || self.executor.is_running(*runtime_id)
                        || self.shortcut_executor.is_running(*runtime_id)
                });
                self.sync_bindings();
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

    pub(super) fn select_wheel(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(wheel) = self.wheels.wheels.iter().find(|wheel| wheel.id == id) else {
            return;
        };
        let name = wheel.name.clone();
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
        self.wheel_slot_selected = Some(index);
        self.wheel_slot_label
            .update(cx, |input, cx| input.set_text(slot.label, cx));
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
        proposed.wheels[index].slots.push(WheelSlot {
            label,
            action: ActionSpec::SendShortcut(shortcut),
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
        if self.bindings.iter().any(|binding| matches!(&binding.action, ActionSpec::ShowWheel(reference) if reference.wheel_id == id)) {
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
        proposed.wheels[index].slots.push(WheelSlot {
            label: binding.message.chars().take(30).collect(),
            action: binding.action,
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
        let Some(id) = self
            .pending
            .as_ref()
            .map(|spec| spec.id)
            .or(self.next_binding_id)
        else {
            self.error = Some("触发器编号已用尽。".into());
            cx.notify();
            return;
        };
        if self.pending.is_none() {
            self.next_binding_id = id.checked_add(1);
        }
        self.clear_pending();
        self.edited_spec = None;
        let spec = crate::TriggerSpec {
            id,
            held: vec![Button::Key(0x14)],
            activation: Activation::Press(Button::Key(0x09)),
        };
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
        self.status = "已生成轮盘呼出草稿；可重新录制组合，保存后生效。".into();
        cx.notify();
    }

    pub(super) fn preview_wheel(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.wheel_selected.clone() {
            self.open_wheel(&id, cx);
        }
    }

    pub(super) fn hide_wheel(&mut self, cx: &mut Context<Self>) {
        self.wheel_generation = self.wheel_generation.wrapping_add(1);
        if let Some(wheel) = self.wheel_window {
            let _ = wheel.update(cx, |view, window, _| view.hide(window));
        }
        self.wheel_active = None;
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
        if already_open {
            cx.notify();
            return;
        }
        let target = match omnitrigger_windows::capture_shortcut_target() {
            Ok(target) => target,
            Err(error) => { self.error = Some(error); cx.notify(); return; }
        };
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
                let Some(runtime_id) = self.next_binding_id else {
                    self.error = Some("动作编号已用尽。".into());
                    cx.notify();
                    return;
                };
                self.next_binding_id = runtime_id.checked_add(1);
                self.wheel_runtime_ids
                    .insert((id.to_string(), index), runtime_id);
            }
        }
        let states = self.wheel_slot_states(&definition);
        let generation = self.wheel_generation;
        let wheel_id = id.to_string();
        let result = window.update(cx, |view, window, cx| {
            view.present(definition, generation, states, window, cx);
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
                .update(cx, |this, _| {
                    this.wheel_generation == generation && this.wheel_active.is_some()
                        && this.wheel_target == Some(target) && this.ready && !this.paused && !this.recording
                })
                .unwrap_or(false);
            if !allowed {
                return;
            }
            let shown = if omnitrigger_windows::capture_shortcut_target().ok() != Some(target) {
                Err("前台窗口已切换，已取消打开轮盘。".into())
            } else {
                omnitrigger_windows::show_wheel_at_cursor_with_size(
                    handle, WHEEL_SIZE, WHEEL_SIZE,
                )
            };
            let should_hide = this.update(cx, |this, cx| {
                if this.wheel_generation != generation {
                    return window.update(cx, |view, _, _| !view.is_visible()).unwrap_or(true);
                }
                match shown {
                    Ok(()) if omnitrigger_windows::capture_shortcut_target().ok() == Some(target) => {
                        this.wheel_registered = Some(handle);
                        this.send(BackendCommand::SetPanelWindow(Some(handle)));
                        this.status = "轨道轮盘已打开：移动选择后点击，或用数字键执行。".into();
                    }
                    Ok(()) => { this.hide_wheel(cx); }
                    Err(error) => {
                        this.error = Some(error);
                        this.hide_wheel(cx);
                    }
                }
                cx.notify();
                this.wheel_active.is_none()
            }).unwrap_or(true);
            if should_hide { omnitrigger_windows::hide_panel_window(handle); }
        })
        .detach();
        cx.notify();
    }

    pub(super) fn refresh_wheel(&self, cx: &mut Context<Self>) {
        if let (Some(id), Some(window)) = (&self.wheel_active, self.wheel_window) {
            if let Some(definition) = self.wheels.wheels.iter().find(|wheel| &wheel.id == id) {
                let states = self.wheel_slot_states(definition);
                let _ = window.update(cx, |view, _, cx| view.update_states(states, cx));
            }
        }
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
                    let target_unchanged = self.wheel_target.is_some()
                        && omnitrigger_windows::capture_shortcut_target().ok() == self.wheel_target;
                    let selected = self
                        .wheels
                        .wheels
                        .iter()
                        .find(|wheel| wheel.id == wheel_id)
                        .and_then(|wheel| {
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
                    if let (Some((name, entry)), Some(id)) = (selected, runtime_id) {
                        if let Some(reason) = self.action_unavailable_reason(&entry.action) {
                            self.error = Some(reason);
                            continue;
                        }
                        if self.action_busy(id, &entry.action) {
                            self.error = Some("此轮盘动作仍在执行，请稍后重试。".into());
                            continue;
                        }
                        let label = format!("{name} · {}", entry.label);
                        if matches!(entry.action, ActionSpec::Notification) {
                            self.show_message(Vec::new(), label, HitSource::Wheel, cx);
                        } else {
                            self.record_hit(Vec::new(), label, HitSource::Wheel);
                            self.dispatch_action(id, entry.action, cx);
                        }
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
        if self.wheel_active.is_some() && !visible {
            self.hide_wheel(cx);
            changed = true;
        } else if visible
            && (self.recording
                || self.paused
                || !self.ready
                || self.wheel_target.is_some_and(|target| {
                    omnitrigger_windows::capture_shortcut_target().ok() != Some(target)
                }))
        {
            self.hide_wheel(cx);
            changed = true;
        }
        changed
    }
}
