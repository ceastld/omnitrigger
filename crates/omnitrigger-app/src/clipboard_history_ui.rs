//! 文本历史页及其专用后台适配。构造不监听；存储、停止与join只在后台执行。

use crate::clipboard_history_background::{Cancellation, HistoryBackground};
use crate::clipboard_history_controller::{
    CommitDisposition, DropReason, Failure, Status, StoreFailure,
};
use crate::clipboard_history_filter::{
    summary_keyboard_action, summary_label, HistoryQueryView, SummaryDirection, SummaryEnterLatch,
    SummaryKeyboardAction, SummaryQuery, MAX_QUERY_CHARACTERS,
};
use crate::clipboard_history_send::{
    HistoryLaunchTarget, HistorySendRequest, HistorySendSession, SendFailure, SendReceipt,
};
use crate::clipboard_history_store::{
    ClipboardHistoryStore, HistoryError, HistoryList, HistorySearch, MAX_ITEMS,
};
use crate::clipboard_history_ui_state::{Control, HistoryUiState, Phase};
use crate::event_bridge::{EventBridge, UiWake};
use crate::message_input::MessageInput;
use crate::theme::{self, button, rgb, Tone};
use gpui::{
    div, prelude::*, px, Context, Div, Entity, FocusHandle, KeyDownEvent, KeyUpEvent, MouseButton,
    ScrollHandle, Subscription, Window,
};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

enum Request {
    Control {
        id: u64,
        command: Control,
        cancellation: Cancellation,
    },
    List {
        id: u64,
    },
    Search {
        id: u64,
        query: SummaryQuery,
    },
    Remove {
        id: u64,
        item: String,
    },
    Send {
        id: u64,
        request: Box<HistorySendRequest>,
    },
}

enum Reply {
    Control {
        id: u64,
        status: Result<Status, ()>,
        message: Option<&'static str>,
    },
    List {
        id: Option<u64>,
        result: Result<HistoryList, &'static str>,
    },
    Search {
        id: u64,
        result: Result<HistorySearch, &'static str>,
    },
    Notice(&'static str),
    Sent {
        id: u64,
        result: Result<SendReceipt, SendFailure>,
    },
}

struct Worker {
    requests: mpsc::SyncSender<Request>,
    replies: EventBridge<Reply>,
    shutdown: Cancellation,
}

impl Drop for Worker {
    fn drop(&mut self) {
        // 不在GPUI线程等待。接收端释放也会解除后台发送的背压。
        self.shutdown.cancel();
    }
}

pub(super) struct ClipboardHistoryUi {
    state: HistoryUiState,
    sending: HistorySendSession,
    query: HistoryQueryView,
    search: Entity<MessageInput>,
    list_focus: FocusHandle,
    list_blur: Option<Subscription>,
    enter_send: SummaryEnterLatch,
    scroll: ScrollHandle,
    _search_changed: Subscription,
    store: Option<Arc<ClipboardHistoryStore>>,
    wake: UiWake,
    worker: Option<Worker>,
    collection: Cancellation,
}

impl ClipboardHistoryUi {
    /// 这里只保存组合根提供的store；不创建线程、不读磁盘、不启用监听。
    pub fn new(
        store: Option<Arc<ClipboardHistoryStore>>,
        wake: UiWake,
        launch_target: Option<HistoryLaunchTarget>,
        ui_window: Option<isize>,
        cx: &mut Context<super::Workspace>,
    ) -> Self {
        let search = cx.new(|cx| {
            let mut input = MessageInput::new(cx).with_character_limit(MAX_QUERY_CHARACTERS);
            input.set_placeholder("筛选摘要；点击“查找正文”搜索完整文本");
            input
        });
        let search_changed = cx.observe(&search, |this, search, cx| {
            match SummaryQuery::new(&search.read(cx).text()) {
                Ok(query) if &query != this.text_history.query.draft() => {
                    this.text_history.cancel_send();
                    this.text_history.enter_send.cancel();
                    this.text_history.query.set_draft(query);
                    this.text_history.scroll.scroll_to_item(0);
                    this.text_history.state.selected = None;
                }
                Ok(_) => {}
                Err(_) => this.text_history.state.message = Some("筛选词过长，请缩短后重试。"),
            }
            cx.notify();
        });
        let collection = Cancellation::default();
        collection.cancel();
        Self {
            state: HistoryUiState::new(store.is_some()),
            sending: HistorySendSession::new(launch_target, ui_window),
            query: HistoryQueryView::default(),
            search,
            list_focus: cx.focus_handle(),
            list_blur: None,
            enter_send: SummaryEnterLatch::default(),
            scroll: ScrollHandle::new(),
            _search_changed: search_changed,
            store,
            wake,
            worker: None,
            collection,
        }
    }

    fn ensure_worker(&mut self) -> bool {
        if self.worker.is_some() {
            return true;
        }
        let Some(store) = self.store.clone() else {
            self.state.disconnected();
            return false;
        };
        let (requests, receive) = mpsc::sync_channel(8);
        let (send, replies) = mpsc::sync_channel(8);
        let Ok(replies) = EventBridge::with_wake(replies, self.wake.clone()) else {
            self.state.disconnected();
            return false;
        };
        let shutdown = Cancellation::default();
        let stop = shutdown.clone();
        let started = thread::Builder::new()
            .name("omnitrigger-text-history-ui".into())
            .spawn(move || run_worker(store, receive, send, stop));
        if started.is_err() {
            self.state.disconnected();
            return false;
        }
        // 句柄不在GUI Drop中join；所有原生监听句柄只由此线程持有和释放。
        self.worker = Some(Worker {
            requests,
            replies,
            shutdown,
        });
        true
    }

    fn submit(&mut self, request: Request) {
        if !self.ensure_worker() {
            self.cancel_send();
            self.collection.cancel();
            return;
        }
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.requests.try_send(request).is_err())
        {
            self.cancel_send();
            self.collection.cancel();
            self.worker.take();
            self.state.disconnected();
        }
    }

    fn primary(&mut self) {
        self.enter_send.cancel();
        if matches!(
            self.state.phase,
            Phase::Starting | Phase::On | Phase::CloseFailed
        ) {
            // 原子取消可穿过正在启动或提交的后台调用；按钮回调不拿提交门或等待stop。
            self.collection.cancel();
        }
        let Some((id, command)) = self.state.primary() else {
            return;
        };
        if command == Control::Enable {
            self.collection = Cancellation::default();
        }
        self.submit(Request::Control {
            id,
            command,
            cancellation: self.collection.clone(),
        });
    }

    /// 页面已明确打开后请求一次摘要；不在构造、render或poll中推断进入。
    pub fn activate(&mut self) {
        self.enter_send.cancel();
        if let Some(id) = self.state.initial_query() {
            self.submit(Request::List { id });
        }
    }

    fn refresh(&mut self) {
        self.enter_send.cancel();
        if let Some(id) = self.state.query() {
            self.submit(Request::List { id });
        }
    }

    fn search_full_text(&mut self, cx: &mut Context<super::Workspace>) {
        self.enter_send.cancel();
        // 直接读本次控件值，不依赖observe是否已经交付，也不读取系统剪贴板。
        let query = match SummaryQuery::new(&self.search.read(cx).text()) {
            Ok(query) => query,
            Err(_) => {
                self.cancel_send();
                self.state.selected = None;
                self.state.message = Some("查找词过长，未查询正文，请缩短后重试。");
                return;
            }
        };
        if &query != self.query.draft() {
            self.cancel_send();
            self.state.selected = None;
            self.scroll.scroll_to_item(0);
            self.query.set_draft(query);
        }
        let Some(id) = self.state.query() else {
            return;
        };
        self.cancel_send();
        self.state.selected = None;
        let query = self.query.begin(id);
        self.submit(Request::Search { id, query });
        if self.worker.is_none() {
            self.query.disconnected();
        }
    }

    pub fn cancel_send(&self) {
        self.sending.cancel();
    }

    fn send_selected(&mut self) {
        self.enter_send.cancel();
        if self.sending.pending() {
            self.cancel_send();
            return;
        }
        if !self.sending.ready()
            || !self
                .query
                .selected_visible(&self.state.items, self.state.selected.as_deref())
        {
            return;
        }
        let Some((id, selected)) = self.state.begin_send_selection() else {
            return;
        };
        match self.sending.begin(id, selected) {
            Ok(request) => self.submit(Request::Send {
                id,
                request: Box::new(request),
            }),
            Err(error) => self.state.sent(id, Err(error)),
        }
    }

    fn select_adjacent(&mut self, direction: SummaryDirection) {
        if !self.state.can_query() {
            return;
        }
        let Some(id) = self
            .query
            .adjacent_id(&self.state.items, self.state.selected.as_deref(), direction)
            .map(str::to_owned)
        else {
            return;
        };
        let Some(visible_index) = self
            .query
            .matching_indices(&self.state.items)
            .iter()
            .position(|&index| self.state.items[index].id == id)
        else {
            return;
        };
        self.sending.selection_changing(&id);
        self.state.select(&id);
        self.scroll.scroll_to_item(visible_index);
    }

    fn focus_list(&mut self, window: &mut Window, cx: &mut Context<super::Workspace>) {
        self.enter_send.cancel();
        if self.list_blur.is_none() {
            self.list_blur = Some(cx.on_blur(&self.list_focus.clone(), window, |this, _, _| {
                this.text_history.enter_send.cancel();
            }));
        }
        self.list_focus.focus(window);
    }

    /// 只由拥有焦点的列表处理；筛选输入是兄弟元素，其编辑/IME事件不进入这里。
    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window) -> bool {
        if !self.list_focus.is_focused(window) {
            self.enter_send.cancel();
            return false;
        }
        let modifiers = &event.keystroke.modifiers;
        let modified = modifiers.shift
            || modifiers.control
            || modifiers.alt
            || modifiers.platform
            || modifiers.function;
        if modified || event.keystroke.key != "enter" || !event.is_held {
            self.enter_send.cancel();
        }
        let can_send = self.state.can_send()
            && self.sending.ready()
            && !self.sending.pending()
            && self
                .query
                .selected_visible(&self.state.items, self.state.selected.as_deref());
        let Some(action) = summary_keyboard_action(
            &event.keystroke.key,
            event.is_held,
            modified,
            self.state.can_query(),
            can_send,
        ) else {
            return event.keystroke.key == "enter" && !modified && self.enter_send.is_armed();
        };
        match action {
            SummaryKeyboardAction::Navigate(direction) => self.select_adjacent(direction),
            SummaryKeyboardAction::ArmSend => {
                if let (Some(id), Some(revision)) = (&self.state.selected, &self.state.revision) {
                    self.enter_send.arm(id, revision);
                }
            }
        }
        true
    }

    fn key_up(&mut self, event: &KeyUpEvent, window: &mut Window) -> bool {
        if event.keystroke.key != "enter" {
            self.enter_send.cancel();
            return false;
        }
        let owned_press = self.enter_send.is_armed();
        let modifiers = &event.keystroke.modifiers;
        let modified = modifiers.shift
            || modifiers.control
            || modifiers.alt
            || modifiers.platform
            || modifiers.function;
        let allowed = self.list_focus.is_focused(window)
            && !modified
            && self.state.can_send()
            && self.sending.ready()
            && !self.sending.pending()
            && self
                .query
                .selected_visible(&self.state.items, self.state.selected.as_deref());
        if self.enter_send.release(
            self.state.selected.as_deref(),
            self.state.revision.as_deref(),
            allowed,
        ) {
            // 收到Enter释放事件后，仍复用原目标/读取/版本/取消校验。
            self.send_selected();
        }
        owned_press
    }

    fn remove_selected(&mut self) {
        self.enter_send.cancel();
        if !self
            .query
            .selected_visible(&self.state.items, self.state.selected.as_deref())
        {
            return;
        }
        if let Some((id, item)) = self.state.remove() {
            self.submit(Request::Remove { id, item });
        }
    }

    /// 复用Workspace既有唤醒与兜底poll；一次最多收32项，不做磁盘或原生输入工作。
    pub fn request_popup_close(&mut self) {
        self.enter_send.cancel();
        self.cancel_send();
        self.collection.cancel();
        if let Some((id, command)) = self.state.request_popup_close() {
            self.submit(Request::Control {
                id,
                command,
                cancellation: self.collection.clone(),
            });
        }
    }

    pub fn popup_close_requested(&self) -> bool {
        self.state.popup_close_requested()
    }

    pub fn popup_close_ready(&self) -> bool {
        self.state.popup_close_ready()
    }

    pub fn poll(&mut self) -> bool {
        let mut changed = self.sending.poll();
        for _ in 0..32 {
            let Some(worker) = self.worker.as_ref() else {
                break;
            };
            match worker.replies.try_recv() {
                Ok(Reply::Control {
                    id,
                    status: Ok(status),
                    message,
                }) => {
                    self.state.acknowledge(id, status, message);
                    changed = true;
                }
                Ok(Reply::Control {
                    id,
                    status: Err(()),
                    ..
                }) => {
                    self.collection.cancel();
                    self.state.control_failed(id);
                    changed = true;
                }
                Ok(Reply::List { id, result }) => {
                    self.state.listed(id, result);
                    if self.query.invalidate(self.state.revision.as_deref()) {
                        if !self
                            .query
                            .selected_visible(&self.state.items, self.state.selected.as_deref())
                        {
                            self.state.selected = None;
                        }
                        if self.state.message.is_none() {
                            self.state.message =
                                Some("历史已变化，已返回摘要筛选；可重新查找正文。");
                        }
                    }
                    changed = true;
                }
                Ok(Reply::Search { id, result }) => {
                    if !self.query.is_pending(id) {
                        continue;
                    }
                    match result {
                        Ok(found) => {
                            let revision = found.list.revision.clone();
                            self.state.listed(Some(id), Ok(found.list));
                            let applied = self.query.complete(
                                id,
                                &revision,
                                self.state.revision.as_deref(),
                                self.state.can_query(),
                                found.matching_ids,
                            );
                            if applied {
                                if !self.query.selected_visible(
                                    &self.state.items,
                                    self.state.selected.as_deref(),
                                ) {
                                    self.state.selected = None;
                                }
                            } else if self.state.can_query() && self.state.message.is_none() {
                                self.state.message = Some("查找词已变化，请重新查找正文。");
                            }
                        }
                        Err(message) => {
                            self.state.listed(Some(id), Err(message));
                            self.query.failed(id);
                        }
                    }
                    changed = true;
                }
                Ok(Reply::Sent { id, result }) => {
                    self.sending.complete(id, &result);
                    self.state.sent(id, result);
                    changed = true;
                }
                Ok(Reply::Notice(message)) => {
                    self.state.message = Some(message);
                    changed = true;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.cancel_send();
                    self.collection.cancel();
                    self.worker.take();
                    self.state.disconnected();
                    self.query.disconnected();
                    changed = true;
                    break;
                }
            }
        }
        changed
    }
}

impl Drop for ClipboardHistoryUi {
    fn drop(&mut self) {
        self.cancel_send();
        self.collection.cancel();
        if let Some(worker) = &self.worker {
            worker.shutdown.cancel();
        }
    }
}

fn operation_check(stop: &Cancellation, deadline: Instant) -> Result<(), HistoryError> {
    if stop.is_cancelled() {
        Err(HistoryError::new("cancelled"))
    } else if Instant::now() >= deadline {
        Err(HistoryError::new("timeout"))
    } else {
        Ok(())
    }
}

fn summaries(
    store: &ClipboardHistoryStore,
    stop: &Cancellation,
) -> Result<HistoryList, &'static str> {
    let deadline = Instant::now() + Duration::from_secs(3);
    store
        .list(MAX_ITEMS, || operation_check(stop, deadline))
        .map_err(|_| "无法读取文本历史，请稍后明确刷新。")
}

fn report_message(reports: &[Failure]) -> Option<&'static str> {
    if reports.contains(&Failure::StopFailed) {
        Some("尚未确认采集已关闭，请重试关闭。")
    } else if reports.contains(&Failure::StartFailed) {
        Some("未能开启采集。")
    } else if reports.contains(&Failure::UnexpectedStop) {
        Some("采集意外停止，请检查状态后明确开启。")
    } else if reports.contains(&Failure::CounterExhausted) {
        Some("采集会话已达上限，请重新打开应用。")
    } else {
        None
    }
}

fn run_worker(
    store: Arc<ClipboardHistoryStore>,
    requests: mpsc::Receiver<Request>,
    replies: mpsc::SyncSender<Reply>,
    shutdown: Cancellation,
) {
    let mut background = HistoryBackground::for_store_with_worker(
        store.clone(),
        omnitrigger_windows::HistoryWorkerBudgets {
            startup_ms: 60_000,
            read_ms: 1_000,
            stop_grace_ms: 500,
        },
    );
    let mut control_id = 0;
    let mut reported_status = Status::Stopped;
    'work: while !shutdown.is_cancelled() {
        match requests.recv_timeout(Duration::from_millis(25)) {
            Ok(Request::Control {
                id,
                command,
                cancellation,
            }) => {
                control_id = id;
                let result = match command {
                    Control::Enable => background.enable(cancellation),
                    Control::Disable => background.disable(),
                    Control::RetryCleanup => background.retry_cleanup(),
                };
                let (status, message) = match result {
                    Ok(step) => {
                        reported_status = step.status;
                        (Ok(step.status), report_message(&step.reports))
                    }
                    Err(_) => (Err(()), None),
                };
                if replies
                    .send(Reply::Control {
                        id,
                        status,
                        message,
                    })
                    .is_err()
                {
                    break;
                }
            }
            Ok(Request::List { id }) => {
                if replies
                    .send(Reply::List {
                        id: Some(id),
                        result: summaries(&store, &shutdown),
                    })
                    .is_err()
                {
                    break;
                }
            }
            Ok(Request::Search { id, query }) => {
                let deadline = Instant::now() + Duration::from_secs(3);
                let result = store
                    .search_full_text(query.terms(), || operation_check(&shutdown, deadline))
                    .map_err(|_| "无法查找完整文本，请明确重试；旧摘要仅供参考。");
                if replies.send(Reply::Search { id, result }).is_err() {
                    break;
                }
            }
            Ok(Request::Remove { id, item }) => {
                let deadline = Instant::now() + Duration::from_secs(3);
                let result = match store.remove(&item, || operation_check(&shutdown, deadline)) {
                    Ok(_) => summaries(&store, &shutdown)
                        .map_err(|_| "删除已完成，但列表未能刷新。请明确刷新后继续。"),
                    Err(error) if error.effect_state == "unknown" => {
                        Err("删除是否生效尚不确定，请先刷新，不要重复删除。")
                    }
                    Err(_) => Err("无法删除选中项，请先刷新后再明确操作。"),
                };
                if replies
                    .send(Reply::List {
                        id: Some(id),
                        result,
                    })
                    .is_err()
                {
                    break;
                }
            }
            Ok(Request::Send { id, request }) => {
                if shutdown.is_cancelled() {
                    break;
                }
                let result = (*request).run(store.clone());
                if replies.send(Reply::Sent { id, result }).is_err() {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if shutdown.is_cancelled() {
            break;
        }
        if !matches!(background.status(), Ok(Status::Running { .. })) {
            continue;
        }
        let step = match background.poll_once() {
            Ok(step) => step,
            Err(_) => {
                let _ = background.cleanup_owned();
                let _ = replies.send(Reply::Control {
                    id: control_id,
                    status: Err(()),
                    message: None,
                });
                break;
            }
        };
        if step.dropped == Some(DropReason::InvalidText)
            && replies
                .send(Reply::Notice("本次文本格式无效，未记录。"))
                .is_err()
        {
            break;
        }
        let message = report_message(&step.reports);
        if step.status != reported_status || message.is_some() {
            reported_status = step.status;
            if replies
                .send(Reply::Control {
                    id: control_id,
                    status: Ok(step.status),
                    message,
                })
                .is_err()
            {
                break;
            }
        }
        match step.commit {
            Some(CommitDisposition::Stored) => {
                if replies
                    .send(Reply::List {
                        id: None,
                        result: summaries(&store, &shutdown),
                    })
                    .is_err()
                {
                    break;
                }
            }
            Some(CommitDisposition::Failed(failure)) => {
                let message = match failure {
                    StoreFailure::Unknown => "本次记录是否写入尚不确定，请明确刷新；不会自动重试。",
                    StoreFailure::Conflict => "历史文件已发生变化，本次记录未写入。请明确刷新。",
                    StoreFailure::Unavailable => "历史存储暂时不可用，本次记录未写入。",
                    StoreFailure::Cancelled => continue 'work,
                };
                if replies.send(Reply::Notice(message)).is_err() {
                    break;
                }
            }
            Some(CommitDisposition::Discarded) | None => {}
        }
    }
    // 即使控制门失败，也只在这个后台线程收尾自己持有的监听器。
    let _ = background.disable();
    let _ = background.cleanup_owned();
}

impl super::Workspace {
    pub(super) fn render_text_history(&self, cx: &mut Context<Self>) -> Div {
        let state = &self.text_history.state;
        let subtitle = match state.phase {
            Phase::Off => "采集已关闭。已有历史仍可查询与删除。",
            Phase::Starting => "正在开启采集…",
            Phase::On => "采集中，仅记录开启后新复制的纯文本。",
            Phase::Closing => "正在关闭采集并等待后台完成…",
            Phase::CloseFailed => "关闭尚未确认，请重试关闭。",
        };
        let mut list = div()
            .id("text-history-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.text_history.scroll)
            .track_focus(&self.text_history.list_focus)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.text_history.focus_list(window, cx);
                }),
            )
            .on_key_down(cx.listener(|this, event, window, cx| {
                if this.text_history.key_down(event, window) {
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .on_key_up(cx.listener(|this, event, window, cx| {
                if this.text_history.key_up(event, window) {
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .rounded(px(14.))
            .border_1()
            .border_color(rgb(theme::BORDER))
            .bg(rgb(theme::ROW));
        let visible = self.text_history.query.matching_indices(&state.items);
        for &index in &visible {
            let item = &state.items[index];
            let selected = state.selected.as_deref() == Some(item.id.as_str());
            let id = item.id.clone();
            list = list.child(
                div()
                    .id(("text-history-row", index))
                    .px(px(16.))
                    .py(px(12.))
                    .border_b_1()
                    .border_color(rgb(theme::DIVIDER))
                    .bg(rgb(if selected { theme::SURFACE } else { theme::ROW }))
                    .hover(|style| style.bg(rgb(theme::SURFACE)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.text_history.focus_list(window, cx);
                        this.text_history.sending.selection_changing(&id);
                        this.text_history.state.select(&id);
                        cx.notify();
                    }))
                    .child(div().truncate().child(summary_label(item))),
            );
        }
        if visible.is_empty() {
            list = list.child(div().p(px(32.)).child(super::caption(
                if self.text_history.query.loading() {
                    "正在查找完整文本…"
                } else if state.initial_list_loading() {
                    "正在加载历史摘要…"
                } else if self.text_history.query.full_text_active() {
                    "完整文本中没有匹配项。"
                } else if state.items.is_empty() && state.revision.is_some() {
                    "暂无历史摘要。"
                } else if state.items.is_empty() {
                    "暂无摘要，点击刷新查询已有历史。"
                } else {
                    "没有匹配的摘要，试试其他筛选词。"
                },
            )));
        }
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .pt(px(6.))
            .pb(px(16.))
            .pl(px(20.))
            .pr(px(24.))
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(
                div()
                    .flex()
                    .items_end()
                    .justify_between()
                    .gap(px(12.))
                    .child(super::heading("文本历史", subtitle))
                    .child(
                        button(
                            "text-history-primary",
                            state.primary_label(),
                            Tone::Primary,
                            state.primary_enabled(),
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.text_history.primary();
                            cx.notify();
                        })),
                    ),
            )
            .child(div().flex_none().child(self.text_history.search.clone()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        button(
                            "text-history-previous",
                            "上一条",
                            Tone::Secondary,
                            state.can_query()
                                && self
                                    .text_history
                                    .query
                                    .adjacent_id(
                                        &state.items,
                                        state.selected.as_deref(),
                                        SummaryDirection::Previous,
                                    )
                                    .is_some(),
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.text_history.focus_list(window, cx);
                            this.text_history
                                .select_adjacent(SummaryDirection::Previous);
                            cx.notify();
                        })),
                    )
                    .child(
                        button(
                            "text-history-next",
                            "下一条",
                            Tone::Secondary,
                            state.can_query()
                                && self
                                    .text_history
                                    .query
                                    .adjacent_id(
                                        &state.items,
                                        state.selected.as_deref(),
                                        SummaryDirection::Next,
                                    )
                                    .is_some(),
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.text_history.focus_list(window, cx);
                            this.text_history.select_adjacent(SummaryDirection::Next);
                            cx.notify();
                        })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        button(
                            "text-history-search-body",
                            if self.text_history.query.loading() {
                                "正在查找…"
                            } else {
                                "查找正文"
                            },
                            Tone::Secondary,
                            state.can_query(),
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.text_history.search_full_text(cx);
                            cx.notify();
                        })),
                    )
                    .child(
                        button(
                            "text-history-refresh",
                            "刷新",
                            Tone::Secondary,
                            state.can_query(),
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.text_history.refresh();
                            cx.notify();
                        })),
                    )
                    .child(
                        button(
                            "text-history-remove",
                            "删除选中项",
                            Tone::Danger,
                            state.can_remove()
                                && self
                                    .text_history
                                    .query
                                    .selected_visible(&state.items, state.selected.as_deref()),
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.text_history.remove_selected();
                            cx.notify();
                        })),
                    )
                    .child(
                        button(
                            "text-history-send",
                            self.text_history.sending.action_label(),
                            Tone::Secondary,
                            self.text_history.sending.action_enabled(
                                state.can_send()
                                    && self
                                        .text_history
                                        .query
                                        .selected_visible(&state.items, state.selected.as_deref()),
                            ),
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.text_history.send_selected();
                            cx.notify();
                        })),
                    )
                    .child(super::caption(format!(
                        "{} {} / {} 条",
                        if self.text_history.query.full_text_active() {
                            "正文命中"
                        } else {
                            "摘要筛选"
                        },
                        visible.len(),
                        state.items.len()
                    ))),
            )
            .when_some(state.message, |element, message| {
                element.child(super::caption(message))
            })
            .child(super::caption(self.text_history.sending.status()))
            .child(super::caption(
                "点击列表后：↑↓ / Home / End 选择，Enter 发送选中项。",
            ))
            .child(list)
    }
}
