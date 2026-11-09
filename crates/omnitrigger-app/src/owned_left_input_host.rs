//! 绑定原DesktopHost目标和当前Backend实例；不会在释放时重选新Hook/前台。
use omnitrigger_actions::{
    owned_mouse::{self, OwnedInputOwner, OwnedLeftBackend, OwnedLeftSession},
    ActionError,
};
use omnitrigger_windows::{OwnedLeftInputClient, ScriptTarget};
use std::{sync::Arc, time::Instant};
struct Backend {
    target: ScriptTarget,
    client: OwnedLeftInputClient,
}
impl OwnedLeftBackend for Backend {
    fn check_original_target(&self) -> Result<(), ActionError> {
        self.target
            .ensure_foreground()
            .map_err(|_| owned_mouse::fault(owned_mouse::DOWN, "target_changed", false))
    }
    fn down_owned(
        &self,
        owner: OwnedInputOwner,
        deadline: Instant,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(), ActionError> {
        let target = self
            .target
            .snapshot()
            .map_err(|_| owned_mouse::fault(owned_mouse::DOWN, "target_changed", false))?;
        self.client
            .down(owner.id(), target, deadline, cancelled)
            .map_err(|error| owned_mouse::fault(owned_mouse::DOWN, error.code, true))
    }
    fn release_owned(&self, owner: OwnedInputOwner) -> Result<(), ActionError> {
        self.client
            .release_owned(owner.id())
            .map_err(|error| owned_mouse::fault(owned_mouse::UP, error.code, true))
    }
}
pub(super) fn create(
    target: ScriptTarget,
    owner: OwnedInputOwner,
) -> Result<Arc<OwnedLeftSession>, ActionError> {
    let client = omnitrigger_windows::active_owned_left_input_client()
        .ok_or_else(|| owned_mouse::fault(owned_mouse::DOWN, "input_owner_unavailable", false))?;
    Ok(Arc::new(OwnedLeftSession::new(
        owner,
        Arc::new(Backend { target, client }),
    )))
}
