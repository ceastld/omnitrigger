//! 只有ActionService创建作用域；脚本参数不能提供调用、绑定、包或动作身份。
use super::{ActionIdentity, Reservations, CAPACITY, STOP_OTHER_MAX_TIMEOUT_MS};
use crate::{ActionError, CancellationToken, NativeHost};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

const METHOD: &str = "actions.stop_other_instances";

pub(super) struct InvocationScopedHost {
    // 内层本次宿主仍先清理；InvocationTask的_active守卫位于整个host之后。
    inner: Option<Arc<dyn NativeHost>>,
    reservations: Arc<Mutex<Reservations>>,
    active_changed: Arc<Condvar>,
    invocation_id: u64,
}

pub(super) fn scoped_host(
    inner: Option<Arc<dyn NativeHost>>,
    reservations: Arc<Mutex<Reservations>>,
    active_changed: Arc<Condvar>,
    invocation_id: u64,
) -> Arc<dyn NativeHost> {
    Arc::new(InvocationScopedHost {
        inner,
        reservations,
        active_changed,
        invocation_id,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StopOptions {
    timeout_ms: u64,
}

fn fault(code: &str, unknown: bool) -> ActionError {
    let mut error = ActionError::new(code, "停止其他调用未获完整确认；不会自动重试或补发输入。");
    error.effect_state = Some(if unknown { "unknown" } else { "not_started" }.into());
    error.method = Some(METHOD.into());
    error
}

impl InvocationScopedHost {
    fn stop(
        &self,
        params: Value,
        cancellation: &CancellationToken,
        outer_deadline: Option<Instant>,
    ) -> Result<Value, ActionError> {
        let options: StopOptions =
            serde_json::from_value(params).map_err(|_| fault("invalid_input", false))?;
        if !(1..=STOP_OTHER_MAX_TIMEOUT_MS).contains(&options.timeout_ms) {
            return Err(fault("invalid_input", false));
        }
        let local_deadline = Instant::now() + Duration::from_millis(options.timeout_ms);
        let deadline = outer_deadline.map_or(local_deadline, |outer| outer.min(local_deadline));
        let deadline_code = if outer_deadline.is_some_and(|outer| outer <= local_deadline) {
            "timeout"
        } else {
            "stop_timeout"
        };
        let mut state = self
            .reservations
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let current = state
            .entries
            .get(&self.invocation_id)
            .filter(|entry| entry.active)
            .ok_or_else(|| fault("action_inactive", false))?;
        if cancellation.is_cancelled() || current.cancellation.is_cancelled() {
            return Err(fault("cancelled", false));
        }
        if current.single_instance {
            return Err(fault("action_policy_conflict", false));
        }
        let identity = ActionIdentity {
            package_id: current.identity.package_id.clone(),
            action_id: current.identity.action_id.clone(),
        };
        // 校验全部策略后才发出任一取消；不取消当前调用，不根据脚本声明的invocation选目标。
        let peers: Vec<_> = state
            .entries
            .iter()
            .filter(|(id, entry)| {
                **id != self.invocation_id && entry.active && entry.identity == identity
            })
            .map(|(&id, entry)| (id, entry.cancellation.clone(), entry.single_instance))
            .collect();
        debug_assert!(peers.len() < CAPACITY);
        if peers.iter().any(|(_, _, forbidden)| *forbidden) {
            return Err(fault("action_busy", false));
        }
        if Instant::now() >= deadline {
            return Err(fault(deadline_code, false));
        }
        for (_, token, _) in &peers {
            token.cancel();
        }
        let unknown = !peers.is_empty();
        loop {
            let current = state
                .entries
                .get(&self.invocation_id)
                .filter(|entry| entry.active)
                .ok_or_else(|| fault("action_inactive", unknown))?;
            if current.cancellation.is_cancelled() || cancellation.is_cancelled() {
                return Err(fault("cancelled", unknown));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(fault(deadline_code, unknown));
            }
            if peers
                .iter()
                .all(|(id, _, _)| !state.entries.get(id).is_some_and(|entry| entry.active))
            {
                return Ok(
                    json!({"stoppedCount":peers.len(),"effectState":if unknown {"unknown"} else {"not_started"}}),
                );
            }
            // 连接断开产生的桥接令牌与服务令牌不同，短轮询也观察该取消；不延长总期限。
            let (guard, _) = self
                .active_changed
                .wait_timeout(state, remaining.min(Duration::from_millis(10)))
                .unwrap_or_else(|error| error.into_inner());
            state = guard;
        }
    }
}

impl NativeHost for InvocationScopedHost {
    fn native_context_available(&self) -> bool {
        self.inner
            .as_deref()
            .is_some_and(|inner| inner.native_context_available())
    }

    fn action_scope_available(&self) -> bool {
        true
    }

    fn call(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
    ) -> Result<Value, ActionError> {
        if method == METHOD {
            return self.stop(params, cancellation, None);
        }
        match self.inner.as_deref() {
            Some(inner) => inner.call(method, params, cancellation),
            None => {
                let mut error =
                    ActionError::new("desktop_unavailable", "此调用没有提供其他本机能力。");
                error.method = Some(method.into());
                error.effect_state = Some("not_started".into());
                Err(error)
            }
        }
    }

    fn call_with_deadline(
        &self,
        method: &str,
        params: Value,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<Value, ActionError> {
        if method == METHOD {
            return self.stop(params, cancellation, Some(deadline));
        }
        match self.inner.as_deref() {
            Some(inner) => inner.call_with_deadline(method, params, cancellation, deadline),
            None => self.call(method, params, cancellation),
        }
    }
}

#[cfg(test)]
#[path = "service_actions_tests.rs"]
mod tests;
