//! 与平台和界面无关的输入录制与触发器状态机。
//!
//! 录制、注册与运行时匹配共用 [`TriggerSpec`]。平台显式提供事件、时间和屏幕上下文，
//! [`Engine`] 返回消费、触发及必要的补发决策；本 crate 不读取系统时钟或调用平台 API。
//!
//! # 最小事件序列
//!
//! 平台负责交付真实事件并执行决策；引擎只判断是否消费、触发或需要补发。
//!
//! ```
//! use omnitrigger_core::{Activation, Button, Engine, InputEvent, InputKind, TriggerSpec};
//!
//! let caps = Button::Key(0x14);
//! let w = Button::Key(0x57);
//! let mut engine = Engine::new(vec![TriggerSpec {
//!     pointer_scope: Default::default(),
//!     wheel: Default::default(),
//!     keyboard: Default::default(),
//!     process_scope: Default::default(),
//!     id: 1,
//!     held: vec![caps],
//!     activation: Activation::Press(w),
//!     conditions: vec![],
//! }])?;
//! let physical = |kind| InputEvent { kind, injected: false };
//!
//! assert!(engine.process_at(physical(InputKind::Down(caps)), 0).consume);
//! let hit = engine.process_at(physical(InputKind::Down(w)), 10);
//! assert_eq!(hit.triggered, vec![1]);
//! assert!(hit.consume);
//! assert!(engine.process_at(physical(InputKind::Up(w)), 20).consume);
//! assert!(engine.process_at(physical(InputKind::Up(caps)), 30).consume);
//! # Ok::<(), omnitrigger_core::RegistrationError>(())
//! ```

mod engine;
mod model;
mod mouse_dispatch_queue;
mod mouse_modifier_snapshot;
mod process_scope;
mod recorder;
mod regions;
mod registration;
mod scene;
mod wheel_input;

pub use engine::Engine;
pub use model::{
    Activation, ActiveDrag, Axis, Button, Decision, DragEvent, DragPhase, InputEvent, InputKind,
    KeyboardMatching, KeyboardPolicy, MouseButton, RetainedPrefix, TriggerSpec, WheelDirection,
    WheelPolicy,
};
pub use mouse_dispatch_queue::{
    ActiveMouseDispatch, BuiltinMouseSwitch, BusyMousePolicy, DispatchEnablement,
    DroppedMouseDispatch, MAX_BUILTIN_MOUSE_SWITCHES, MAX_DELAYED_DISPATCHES,
    MouseDispatchAdmission, MouseDispatchCancellation, MouseDispatchClass, MouseDispatchControls,
    MouseDispatchDropReason, MouseDispatchError, MouseDispatchQueue, MouseDispatchRequest,
    MouseDispatchUpdate,
};
pub use mouse_modifier_snapshot::{
    ModifierRequirement, ModifierSides, MouseCaptureOwnership, MouseContextEvent,
    MouseModifierContext, MouseModifierGate, MouseModifierInput, MouseModifierPolicy,
    MouseModifierSnapshot, MouseSnapshotError, ObservationOwnership, ObservedPress,
};
pub use process_scope::{
    ProcessIdentity, ProcessScope, ProcessScopeError, ScopeIntersectionError, WindowRole,
    canonical_process_name,
};
pub use recorder::Recorder;
pub use regions::{
    ContextError, PointerRegion, RegionError, ScreenCorner, ScreenEdge, ScreenRect,
    TriggerCondition,
};
pub use registration::{MAX_BINDINGS, RegistrationError};
pub use scene::{Scene, SceneConflict, SceneDimension};
pub use wheel_input::{
    WheelInputAdmission, WheelInputNormalizer, WheelInputPolicy, WheelSessionSnapshot,
};

#[cfg(test)]
mod prefix_tests;

#[cfg(test)]
mod retained_tests;

#[cfg(test)]
mod terminal_role_tests;

#[cfg(test)]
mod captured_matching_tests;

#[cfg(test)]
mod keyboard_policy_tests;

#[cfg(test)]
mod mouse_matching_tests;

#[cfg(test)]
mod mouse_leader_tests;

#[cfg(test)]
mod process_scope_tests;

#[cfg(test)]
mod region_tests;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod wheel_policy_tests;

#[cfg(test)]
mod pointer_scope_tests;
#[cfg(test)]
mod scene_tests;
