//! 文本历史页及其专用后台适配。构造不监听；存储、停止与join只在后台执行。

use crate::clipboard_history_background::{Cancellation, HistoryBackground};
use crate::clipboard_history_controller::{
    CommitDisposition, DropReason, Failure, Status, StoreFailure,
};
use crate::clipboard_history_filter::{summary_label, SummaryQuery, MAX_QUERY_CHARACTERS};
use crate::clipboard_history_store::{ClipboardHistoryStore, HistoryError, HistoryList, MAX_ITEMS};
use crate::clipboard_history_ui_state::{Control, HistoryUiState, Phase};
use crate::event_bridge::{EventBridge, UiWake};
use crate::message_input::MessageInput;
use crate::theme::{self, button, rgb, Tone};
use gpui::{div, prelude::*, px, Context, Div, Entity, Subscription};
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
    Remove {
        id: u64,
        item: String,
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
    Notice(&'static str),
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
    query: SummaryQuery,
    search: Entity<MessageInput>,
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
        cx: &mut Context<super::Workspace>,
    ) -> Self {
        let search = cx.new(|cx| {
            let mut input = MessageInput::new(cx).with_character_limit(MAX_QUERY_CHARACTERS);
            input.set_placeholder("筛选摘要（不搜索正文）");
            input
        });
        let search_changed = cx.observe(&search, |this, search, cx| {
            match SummaryQuery::new(&search.read(cx).text()) {
                Ok(query) if query != this.text_history.query => {
                    this.text_history.query = query;
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
            query: SummaryQuery::default(),
            search,
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
            self.collection.cancel();
            return;
        }
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.requests.try_send(request).is_err())
        {
            self.collection.cancel();
            self.worker.take();
            self.state.disconnected();
        }
    }

    fn primary(&mut self) {
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

    fn refresh(&mut self) {
        if let Some(id) = self.state.query() {
            self.submit(Request::List { id });
        }
    }

    fn remove_selected(&mut self) {
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
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
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
                    changed = true;
                }
                Ok(Reply::Notice(message)) => {
                    self.state.message = Some(message);
                    changed = true;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.collection.cancel();
                    self.worker.take();
                    self.state.disconnected();
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
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.text_history.state.select(&id);
                        cx.notify();
                    }))
                    .child(div().truncate().child(summary_label(item))),
            );
        }
        if visible.is_empty() {
            list = list.child(
                div()
                    .p(px(32.))
                    .child(super::caption(if state.items.is_empty() {
                        "暂无摘要，点击刷新查询已有历史。"
                    } else {
                        "没有匹配的摘要，试试其他筛选词。"
                    })),
            );
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
                    .child(super::caption(format!(
                        "{} / {} 条摘要",
                        visible.len(),
                        state.items.len()
                    ))),
            )
            .when_some(state.message, |element, message| {
                element.child(super::caption(message))
            })
            .child(list)
    }
}
