//! Workspace既有安装事件泵的Legacy分支；不建立第二个manager、channel或业务队列。
use crate::{
    legacy_load_gate::{PrepareResult, PrepareTicket},
    legacy_plugin_work::{
        self as work, LegacyArtifactSource, LegacyCompletion, LegacyFinished, LegacyJob,
        LegacyOperation,
    },
    plugin_install, settings, BackendCommand, Context, Workspace,
};
use omnitrigger_actions::{ActionCall, CancellationToken};
use std::sync::Arc;

pub(super) struct LegacyUiJob {
    ticket: PrepareTicket,
    operation: LegacyOperation,
    cancellation: CancellationToken,
}

// 只根据现有单作业槽与已记录终态决定关闭下一步，不拥有线程/队列/计时器。
#[derive(Debug, PartialEq, Eq)]
enum CloseRoute {
    Quit,
    WaitPrepare,
    WaitStop,
    StartStop,
    Unconfirmed,
}
fn close_route(
    operation: Option<LegacyOperation>,
    session_used: bool,
    last_stop: Option<bool>,
) -> CloseRoute {
    match operation {
        Some(LegacyOperation::Prepare) => CloseRoute::WaitPrepare,
        Some(LegacyOperation::Stop) => CloseRoute::WaitStop,
        None if !session_used || last_stop == Some(true) => CloseRoute::Quit,
        None if last_stop == Some(false) => CloseRoute::Unconfirmed,
        None => CloseRoute::StartStop,
    }
}

impl Workspace {
    /// 正常第一次关闭：先拒新业务，保留窗口/50ms原事件泵直到准备终止和一次stop终态。
    pub(super) fn request_legacy_close(&mut self, cx: &mut Context<Self>) -> bool {
        if self.legacy_close_requested {
            return false;
        }
        self.legacy_close_requested = true;
        self.legacy_gate.disable();
        let route = close_route(
            self.legacy_job.as_ref().map(|job| job.operation),
            self.legacy_session_used,
            self.legacy_last_stop,
        );
        // 默认无Legacy会话直接沿原关闭路径，不提交任何控制命令或附加IO。
        if route == CloseRoute::Quit {
            self.finish_legacy_close(cx);
            return true;
        }
        self.text_history.cancel_send();
        self.send(BackendCommand::SetEnabled(false));
        self.paused = true;
        match route {
            CloseRoute::Quit => {
                self.finish_legacy_close(cx);
                return true;
            }
            CloseRoute::WaitPrepare => {
                if let Some(job) = &self.legacy_job {
                    job.cancellation.cancel();
                }
                self.legacy_stop_after_job = true;
                self.legacy_status = Some("正在退出：等待准备作业结束后正常停止宿主…".into());
            }
            CloseRoute::WaitStop => {
                self.legacy_status = Some("正在退出：等待既有停机请求回执…".into())
            }
            CloseRoute::StartStop => self.start_legacy_stop(cx),
            CloseRoute::Unconfirmed => self.finish_unconfirmed_legacy_close(cx),
        }
        self.try_finish_legacy_close(cx);
        cx.notify();
        false
    }

    fn try_finish_legacy_close(&mut self, cx: &mut Context<Self>) {
        if !self.legacy_close_requested || self.legacy_job.is_some() || self.legacy_stop_after_job {
            return;
        }
        match close_route(None, self.legacy_session_used, self.legacy_last_stop) {
            CloseRoute::Quit => self.finish_legacy_close(cx),
            CloseRoute::Unconfirmed => self.finish_unconfirmed_legacy_close(cx),
            _ => {} // 此函数不重发stop；正常停止只由首次关闭或旧准备终态衔接。
        }
    }
    fn finish_unconfirmed_legacy_close(&mut self, cx: &mut Context<Self>) {
        eprintln!(
            "{}",
            serde_json::json!({"scope":"legacy_shutdown_on_exit","outcome":"unknown",
            "stop_retried":false,"process_exit_verified":false,"detail":self.legacy_status})
        );
        self.finish_legacy_close(cx);
    }
    fn finish_legacy_close(&mut self, cx: &mut Context<Self>) {
        debug_assert!(self.legacy_job.is_none());
        self.text_history.cancel_send();
        self.backend.take(); // 原有最终退出清理；此前保留事件泵，不在UI同步等待Legacy。
        cx.quit();
    }

    /// 启动、刷新或安装worker已完成登记后复用此组合点；不执行磁盘哈希或用户动作。
    pub(super) fn connect_installed_legacy(&mut self, cx: &mut Context<Self>) {
        if !self.mode.owns_input_engine() {
            return;
        }
        match crate::legacy_local_source::locate(&settings::path(), &self.plugins) {
            Ok(Some(source)) => {
                if let Err(error) = self.accept_legacy_artifact_source(source, cx) {
                    self.legacy_status = Some(error);
                    cx.notify();
                } else {
                    // 用户显式刷新允许重新观察同一安装；新来源已排作业时不会重复提交。
                    self.start_legacy_prepare(true, cx);
                }
            }
            Ok(None) => {}
            Err(error) => {
                self.legacy_status = Some(error.to_string());
                cx.notify();
            }
        }
    }

    /// 安装/启动组合根提供既有完整性校验适配；默认没有额外启用或审批状态。
    pub(super) fn accept_legacy_artifact_source(
        &mut self,
        source: Arc<dyn LegacyArtifactSource>,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.legacy_close_requested {
            return Err("应用正在退出，未开始新的宿主准备。".into());
        }
        if !self.mode.owns_input_engine() {
            return Err("当前窗口不拥有插件生命周期。".into());
        }
        if self.install_events.is_some() {
            return Err("请等待当前插件作业结束。".into());
        }
        if self.plugins.legacy_enabled()
            && self
                .legacy_source
                .as_ref()
                .is_some_and(|old| old.key() != source.key())
        {
            return Err("更换宿主版本前须正常停用并核验旧宿主退出。".into());
        }
        let source_changed = self.legacy_source.as_ref().map(|old| old.key()) != Some(source.key());
        self.legacy_source = Some(source);
        if source_changed {
            // 运行中安装/登记完成后同步既有动作目录，不等重启或手动刷新脚本。
            self.refresh_script_catalog();
            self.sync_bindings();
            self.refresh_panel(cx);
            cx.notify();
        }
        if self.plugins.legacy_enabled() {
            self.start_legacy_prepare(false, cx);
        }
        Ok(())
    }

    pub(super) fn toggle_legacy_plugin(&mut self, cx: &mut Context<Self>) {
        if self.legacy_close_requested {
            return;
        }
        if !self.mode.owns_input_engine() {
            return;
        }
        let enabled = !self.plugins.legacy_enabled();
        if self.install_events.is_some() && (enabled || self.legacy_job.is_none()) {
            self.error = Some("请等待当前插件作业结束。".into());
            cx.notify();
            return;
        }
        if enabled && self.legacy_source.is_none() {
            self.error = Some("尚未找到已安装的宿主。".into());
            cx.notify();
            return;
        }
        if let Some(error) = &self.plugin_settings_error {
            self.error = Some(error.clone());
            cx.notify();
            return;
        }
        let mut candidate = self.plugins.clone();
        candidate.configure_legacy_enabled(enabled);
        if let Err(error) = candidate.save(&settings::path()) {
            self.error = Some(error.to_string());
            cx.notify();
            return;
        }
        self.plugins = candidate; // 只持久化本次显式选择；不改原RaiShot默认值或绑定。
        self.error = None;
        if enabled {
            self.start_legacy_prepare(true, cx);
        } else {
            self.legacy_gate.disable(); // 先阻止新业务；取消不等于serve已经停止。
            if let Some(job) = &self.legacy_job {
                if job.operation == LegacyOperation::Prepare {
                    job.cancellation.cancel();
                    self.legacy_stop_after_job = true; // 仅停用控制意图，绝不排队业务/目标。
                    self.legacy_status = Some("已停用，正在结束准备后正常停止宿主…".into());
                }
            } else {
                self.start_legacy_stop(cx);
            }
        }
        self.sync_bindings();
        self.refresh_panel(cx);
        cx.notify();
    }

    fn start_legacy_prepare(&mut self, explicit: bool, cx: &mut Context<Self>) {
        if self.legacy_close_requested
            || self.install_events.is_some()
            || !self.plugins.legacy_enabled()
        {
            return;
        }
        let Some(source) = self.legacy_source.as_ref().cloned() else {
            return;
        };
        match self.legacy_gate.begin(source.key(), explicit) {
            Ok(Some(ticket)) => self.start_legacy_job(ticket, LegacyOperation::Prepare, source, cx),
            Ok(None) => {}
            Err(code) => {
                self.legacy_status = Some(format!("无法准备宿主：{code}"));
                cx.notify();
            }
        }
    }
    fn start_legacy_stop(&mut self, cx: &mut Context<Self>) {
        let Some(source) = self.legacy_source.as_ref().cloned() else {
            self.legacy_last_stop = Some(false);
            self.legacy_status = Some("插件已停用；正式宿主来源未知，未确认宿主停止。".into());
            return;
        };
        match self.legacy_gate.begin_shutdown(source.key()) {
            Ok(ticket) => self.start_legacy_job(ticket, LegacyOperation::Stop, source, cx),
            Err(code) => {
                self.legacy_last_stop = Some(false);
                self.legacy_status = Some(format!("插件已停用；无法请求停机：{code}"));
                cx.notify();
            }
        }
    }
    fn start_legacy_job(
        &mut self,
        ticket: PrepareTicket,
        operation: LegacyOperation,
        source: Arc<dyn LegacyArtifactSource>,
        cx: &mut Context<Self>,
    ) {
        let cancellation = CancellationToken::new();
        let job = LegacyJob {
            ticket: ticket.clone(),
            operation,
            config: settings::path(),
            source,
            cancellation: cancellation.clone(),
        };
        match plugin_install::start_legacy_job(job) {
            Ok(events) => {
                if operation == LegacyOperation::Prepare {
                    self.legacy_session_used = true;
                }
                self.legacy_last_stop = None;
                self.install_events = Some(events);
                self.legacy_job = Some(LegacyUiJob {
                    ticket,
                    operation,
                    cancellation,
                });
                self.legacy_status = Some(
                    match operation {
                        LegacyOperation::Prepare => "正在准备宿主…",
                        LegacyOperation::Stop => "插件已停用，正在请求正常停止宿主…",
                    }
                    .into(),
                );
            }
            Err(error) => {
                if operation == LegacyOperation::Stop {
                    self.legacy_last_stop = Some(false);
                }
                if operation == LegacyOperation::Prepare {
                    self.legacy_gate.finish(
                        &ticket,
                        PrepareResult::NotReady("legacy_worker_unavailable"),
                    );
                }
                self.legacy_status = Some(error);
            }
        }
        cx.notify();
    }

    pub(super) fn finish_legacy_job(&mut self, event: LegacyFinished, cx: &mut Context<Self>) {
        let Some(current) = self.legacy_job.as_ref() else {
            return;
        };
        if current.ticket != event.ticket || current.operation != event.operation {
            return;
        }
        self.legacy_job = None;
        // 收据仅作本次回执来源核对；正常停止会在worker再核验源，不缓存未核验EXE。
        if event
            .installation
            .as_ref()
            .is_some_and(|install| install.key != event.ticket.key)
        {
            self.legacy_gate.disable();
            if event.operation == LegacyOperation::Stop {
                self.legacy_last_stop = Some(false);
            }
            self.error = Some("宿主作业收据不一致。".into());
        } else {
            if event.operation == LegacyOperation::Stop {
                self.legacy_last_stop = Some(matches!(
                    &event.result,
                    Ok(LegacyCompletion::ShutdownAccepted
                        | LegacyCompletion::EndpointAlreadyAbsent
                        | LegacyCompletion::EndpointStopping)
                ));
            }
            match event.result {
                Ok(LegacyCompletion::Prepared(result)) => {
                    if self.plugins.legacy_enabled() && !self.legacy_close_requested {
                        self.legacy_gate.finish(&event.ticket, result);
                        self.legacy_status = Some(match result {
                            PrepareResult::Ready => "宿主已就绪。".into(),
                            PrepareResult::NotReady(code) => format!("宿主未就绪：{code}"),
                        });
                    }
                }
                Ok(LegacyCompletion::EndpointAlreadyAbsent) => {
                    self.legacy_status = Some("插件已停用，宿主端点已不存在。".into())
                }
                Ok(LegacyCompletion::ShutdownAccepted) => {
                    self.legacy_status =
                        Some("插件已停用；宿主已接受正常退出请求，进程退出尚未核验。".into())
                }
                Ok(LegacyCompletion::EndpointStopping) => {
                    self.legacy_status = Some("插件已停用；宿主正在停止，进程退出尚未核验。".into())
                }
                Err(error) => {
                    if event.operation == LegacyOperation::Prepare {
                        self.legacy_gate.finish(
                            &event.ticket,
                            PrepareResult::NotReady("legacy_preparation_failed"),
                        );
                    }
                    self.legacy_status = Some(format!(
                        "{}：{error}",
                        if self.plugins.legacy_enabled() {
                            "宿主准备未完成"
                        } else {
                            "插件已停用，停止未确认"
                        }
                    ));
                    if !(self.legacy_close_requested && error.code == "cancelled") {
                        self.error = Some(error.to_string());
                    }
                }
            }
        }
        if self.legacy_stop_after_job {
            self.legacy_stop_after_job = false;
            self.start_legacy_stop(cx); // 旧作业已结束，使用新token；至多一次正常stop。
        }
        self.sync_bindings();
        self.refresh_panel(cx);
        self.try_finish_legacy_close(cx);
        cx.notify();
    }

    pub(super) fn legacy_worker_disconnected(&mut self, cx: &mut Context<Self>) {
        if let Some(job) = self.legacy_job.take() {
            if job.operation == LegacyOperation::Stop {
                self.legacy_last_stop = Some(false);
            }
            self.legacy_gate.finish(
                &job.ticket,
                PrepareResult::NotReady("legacy_worker_disconnected"),
            );
            self.legacy_status = Some("宿主作业未返回完成回执，未确认运行状态。".into());
            self.error = Some("宿主作业未返回完成回执。".into());
            if self.legacy_stop_after_job {
                self.legacy_stop_after_job = false;
                self.start_legacy_stop(cx);
            }
            self.sync_bindings();
            self.refresh_panel(cx);
            self.try_finish_legacy_close(cx);
            cx.notify();
        }
    }

    pub(super) fn legacy_unavailable_reason(&self, call: &ActionCall) -> Option<String> {
        if call.package_id != work::PACKAGE_ID {
            return None;
        }
        if !self.plugins.legacy_enabled() {
            return Some("历史与聚合窗口插件已停用。".into());
        }
        let Some(source) = &self.legacy_source else {
            return Some("正式宿主尚未核验。".into());
        };
        self.legacy_gate
            .admit(&source.key())
            .err()
            .map(|code| format!("宿主尚未就绪：{code}"))
    }
}

#[cfg(test)]
mod close_tests {
    use super::*;
    #[test]
    fn disabled_unused_session_closes_without_control_work() {
        assert_eq!(close_route(None, false, None), CloseRoute::Quit);
    }
    #[test]
    fn preparing_has_priority_over_an_old_accepted_stop() {
        assert_eq!(
            close_route(Some(LegacyOperation::Prepare), true, Some(true)),
            CloseRoute::WaitPrepare
        );
    }
    #[test]
    fn preparing_then_one_stop_then_acceptance_allows_quit() {
        assert_eq!(
            close_route(Some(LegacyOperation::Prepare), true, None),
            CloseRoute::WaitPrepare
        );
        assert_eq!(close_route(None, true, None), CloseRoute::StartStop);
        assert_eq!(
            close_route(Some(LegacyOperation::Stop), true, None),
            CloseRoute::WaitStop
        );
        assert_eq!(close_route(None, true, Some(true)), CloseRoute::Quit);
    }
    #[test]
    fn failed_stop_requires_notice_instead_of_another_stop() {
        assert_eq!(
            close_route(None, true, Some(false)),
            CloseRoute::Unconfirmed
        );
    }
    #[test]
    fn already_disabled_and_accepted_stop_does_not_stop_again() {
        assert_eq!(close_route(None, true, Some(true)), CloseRoute::Quit);
    }
}
