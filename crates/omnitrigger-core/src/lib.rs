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
mod process_scope;
mod recorder;
mod regions;
mod registration;

pub use engine::Engine;
pub use model::{
    Activation, ActiveDrag, Axis, Button, Decision, DragEvent, DragPhase, InputEvent, InputKind,
    KeyboardMatching, KeyboardPolicy, MouseButton, RetainedPrefix, TriggerSpec, WheelDirection,
};
pub use process_scope::{ProcessIdentity, ProcessScope, ProcessScopeError, WindowRole};
pub use recorder::Recorder;
pub use regions::{
    ContextError, PointerRegion, RegionError, ScreenCorner, ScreenEdge, ScreenRect,
    TriggerCondition,
};
pub use registration::{MAX_BINDINGS, RegistrationError};

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
mod process_scope_tests;

#[cfg(test)]
mod region_tests;

#[cfg(test)]
mod tests;
