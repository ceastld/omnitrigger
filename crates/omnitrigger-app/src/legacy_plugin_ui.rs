//! Workspace既有安装事件泵的Legacy分支；不建立第二个manager、channel或业务队列。
use crate::{
    legacy_load_gate::{LegacyKey, PrepareResult, PrepareTicket},
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

/// 首次安装叶不承担换版；停用或stop受理均不能证明旧进程实际退出。
fn require_same_loaded_key(
    current: Option<&LegacyKey>,
    incoming: &LegacyKey,
) -> Result<(), &'static str> {
    if current.is_some_and(|old| old != incoming) {
        Err("legacy_update_requires_installer")
    } else {
        Ok(())
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
        if self.legacy_installing {
            self.text_history.cancel_send();
            self.send(BackendCommand::SetEnabled(false));
            self.paused = true;
            self.legacy_stop_after_job = self.legacy_session_used;
            self.legacy_status = Some("正在退出：等待安装线程结束，不再启用新宿主…".into());
            cx.notify();
            return false;
        }
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
        if !self.legacy_close_requested
            || self.legacy_installing
            || self.legacy_job.is_some()
            || self.legacy_stop_after_job
        {
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
        debug_assert!(self.legacy_job.is_none() && !self.legacy_installing);
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
        require_same_loaded_key(
            self.legacy_source.as_ref().map(|old| old.key()).as_ref(),
            &source.key(),
        )
        .map_err(|code| {
            format!("{code}：当前窗口不替换已接入版本；停用回包不等于旧宿主实际退出。")
        })?;
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

    /// 与RaiShot同样：显式安装意味着启用；来源仍由组合根/调用者提供。
    pub(super) fn install_legacy_from_source(
        &mut self,
        source: Option<plugin_install::LegacyInstallSource>,
        cx: &mut Context<Self>,
    ) {
        if self.legacy_close_requested || !self.mode.owns_input_engine() {
            return;
        }
        if self.install_events.is_some() || self.legacy_job.is_some() {
            self.error = Some("请等待当前插件作业结束。".into());
            cx.notify();
            return;
        }
        if self.plugin_settings_error.is_some() {
            self.error = Some("请先修复插件设置文件。".into());
            cx.notify();
            return;
        }
        if let Some(source) = source.as_ref() {
            if let Err(error) = source.validate() {
                self.legacy_status = Some(format!("{}：{error}", error.code));
                cx.notify();
                return;
            }
            let incoming = match crate::legacy_local_source::LocalLegacySource::new(
                &settings::path(),
                &source.manifest_sha256,
            ) {
                Ok(source) => source.key(),
                Err(error) => {
                    self.legacy_status = Some(format!("{}：{error}", error.code));
                    cx.notify();
                    return;
                }
            };
            if let Err(code) = require_same_loaded_key(
                self.legacy_source.as_ref().map(|old| old.key()).as_ref(),
                &incoming,
            ) {
                self.legacy_status = Some(format!(
                    "{code}：此窗口仍接入旧版本，未开始安装或改变启用选择。"
                ));
                cx.notify();
                return;
            }
            if self
                .plugins
                .legacy_manifest_sha256()
                .is_some_and(|old| !old.eq_ignore_ascii_case(&source.manifest_sha256))
            {
                self.legacy_status =
                    Some("legacy_update_requires_installer：此入口不能替换已登记版本。".into());
                cx.notify();
                return;
            }
            // 只用原enabled字段记录此刻的显式选择；后台作业不回写该字段。
            match crate::plugins::Plugins::set_legacy_enabled(
                &settings::path(),
                true,
                self.plugins.legacy_manifest_sha256(),
            ) {
                Ok(latest) => self.plugins = latest,
                Err(error) => {
                    self.error = Some(error.to_string());
                    cx.notify();
                    return;
                }
            }
        } // None不写启用选择，由worker返回结构化unavailable。
        match plugin_install::start_legacy_install(settings::path(), source) {
            Ok(events) => {
                self.install_events = Some(events);
                self.legacy_installing = true;
                self.legacy_status = Some("正在核验并安装宿主…".into());
            }
            Err(error) => self.legacy_status = Some(error),
        }
        self.sync_bindings();
        cx.notify();
    }

    pub(super) fn finish_legacy_install(
        &mut self,
        result: Result<Box<work::VerifiedLegacyInstall>, omnitrigger_actions::ActionError>,
        cx: &mut Context<Self>,
    ) {
        self.legacy_installing = false;
        if !self.mode.owns_input_engine() {
            return;
        }
        if self.legacy_close_requested {
            // 关闭期间不登记/启用新包；已有会话只衔接原来的一次stop意图。
            if self.legacy_stop_after_job {
                self.legacy_stop_after_job = false;
                self.start_legacy_stop(cx);
            }
            self.try_finish_legacy_close(cx);
            cx.notify();
            return;
        }
        match result {
            Ok(installed)
                if require_same_loaded_key(
                    self.legacy_source.as_ref().map(|old| old.key()).as_ref(),
                    &installed.key,
                )
                .is_err() =>
            {
                // 先核对当前接入key，再读/写登记；迟到收据不能借停用状态换代。
                self.legacy_status = Some(
                    "legacy_update_requires_installer：迟到收据属于另一版本，未登记或准备。".into(),
                );
            }
            Ok(installed) => match crate::plugins::Plugins::load(&settings::path()) {
                Ok(mut plugins) => {
                    // 此处才在UI事件泵登记，读最新enabled，不使用下载前的快照。
                    match crate::legacy_local_source::register_verified(
                        &settings::path(),
                        &installed,
                        &mut plugins,
                    ) {
                        Ok(_) => {
                            self.plugins = plugins;
                            self.legacy_status = Some(if self.plugins.legacy_enabled() {
                                "宿主已安装并登记，正在准备…".into()
                            } else {
                                "宿主已安装并登记，插件保持停用。".into()
                            });
                            self.connect_installed_legacy(cx);
                        }
                        Err(error) => self.legacy_status = Some(format!("{}：{error}", error.code)),
                    }
                }
                Err(error) => self.legacy_status = Some(error.to_string()),
            },
            Err(error) => self.legacy_status = Some(format!("{}：{error}", error.code)),
        }
        if self.legacy_stop_after_job {
            self.legacy_stop_after_job = false;
            self.start_legacy_stop(cx);
        }
        self.sync_bindings();
        self.refresh_panel(cx);
        cx.notify();
    }

    pub(super) fn toggle_legacy_plugin(&mut self, cx: &mut Context<Self>) {
        if self.legacy_close_requested {
            return;
        }
        if !self.mode.owns_input_engine() {
            return;
        }
        let enabled = !self.plugins.legacy_enabled();
        if self.install_events.is_some()
            && (enabled || (self.legacy_job.is_none() && !self.legacy_installing))
        {
            self.error = Some("请等待当前插件作业结束。".into());
            cx.notify();
            return;
        }
        if enabled && self.legacy_source.is_none() {
            self.install_legacy_from_source(None, cx);
            return;
        }
        if let Some(error) = &self.plugin_settings_error {
            self.error = Some(error.clone());
            cx.notify();
            return;
        }
        let loaded = self.legacy_source.as_ref().map(|source| source.key());
        match crate::plugins::Plugins::set_legacy_enabled(
            &settings::path(),
            enabled,
            loaded.as_ref().map(|key| key.artifact_fingerprint.as_str()),
        ) {
            Ok(latest) => self.plugins = latest,
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
                return;
            }
        }
        self.error = None;
        if enabled {
            self.start_legacy_prepare(true, cx);
        } else {
            self.legacy_gate.disable(); // 先阻止新业务；取消不等于serve已经停止。
            if self.legacy_installing {
                self.legacy_stop_after_job = self.legacy_session_used;
                self.legacy_status = Some("插件已停用，安装会完成但不会启动新宿主。".into());
            } else if let Some(job) = &self.legacy_job {
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
        if !self
            .plugins
            .legacy_pin_matches(Some(&source.key().artifact_fingerprint))
        {
            self.legacy_status = Some("已登记的宿主版本已变更，请重新打开此窗口。".into());
            cx.notify();
            return;
        }
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
                    if self.plugins.legacy_enabled()
                        && !self.legacy_close_requested
                        && self
                            .plugins
                            .legacy_pin_matches(Some(&event.ticket.key.artifact_fingerprint))
                    {
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
        if !self
            .plugins
            .legacy_pin_matches(Some(&source.key().artifact_fingerprint))
        {
            return Some("已登记的宿主版本已变更，请重新打开此窗口。".into());
        }
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
    fn loaded_artifact_key_cannot_change_via_a_late_install_receipt() {
        let current = LegacyKey {
            package_id: work::PACKAGE_ID.into(),
            artifact_fingerprint: "owned-old-fingerprint".into(),
            fixed_scope: "default".into(),
        };
        assert!(require_same_loaded_key(None, &current).is_ok());
        assert!(require_same_loaded_key(Some(&current), &current).is_ok());
        let mut replacement = current.clone();
        replacement.artifact_fingerprint = "owned-new-fingerprint".into();
        // 判定不依赖enabled或stop受理回包，二者都不是进程实际退出凭证。
        assert_eq!(
            require_same_loaded_key(Some(&current), &replacement),
            Err("legacy_update_requires_installer")
        );
        replacement = current.clone();
        replacement.fixed_scope = "other-scope".into();
        assert!(require_same_loaded_key(Some(&current), &replacement).is_err());
    }
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
