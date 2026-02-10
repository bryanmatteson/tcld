//! Wire protocol for tcld's out-of-process debugging.
//!
//! Tier 4. Defines the format-shape of the cross-process surface:
//!
//! - [`frame::WireFrame`] — length-prefixed Tcl-list envelope shared
//!   by every message
//! - [`op::Op`] / [`op::OpResponse`] — closed v2 op vocabulary
//!   mirroring the Tier 0–3 `::tcld::dbg::*` surface
//! - [`event::WireEvent`] — closed v2 event vocabulary mirroring
//!   `tcld_debug_state::DebugEvent` field-for-field
//! - [`error::WireError`] / [`error::ErrorCode`] — closed error code
//!   set
//! - [`handshake::Hello`] / [`handshake::HelloAck`] — protocol
//!   negotiation
//! - [`codec`] — encode/decode framing on any `Read + Write`
//!
//! The crate is pure Rust with no Tcl dependencies. Anyone implementing
//! the wire protocol — Rust controller, Tcl controller, future
//! controllers — speaks against these types.

pub mod codec;
pub mod controller;
pub mod error;
pub mod event;
pub mod frame;
pub mod handshake;
pub mod location;
pub mod op;
pub mod tcl_list;

pub use codec::{decode_frame, encode_frame};
pub use controller::{Controller, ControllerError, ControllerState, NubChild};
pub use error::{ErrorCode, WireError};
pub use event::{PauseReason, WireEvent};
pub use frame::{FrameKind, WireFrame};
pub use handshake::{Hello, HelloAck, NubTclAbi, PROTOCOL_VERSION, validate_nub_tcl_abi};
pub use location::{SourceBlockKind, SourceLocation, SourceLocationParseError};
pub use op::{BreakpointTarget, Op, OpResponse};
pub use tcl_list::{tcl_list_format, tcl_list_parse};

/// Major protocol version. Bumped on breaking changes; parsers
/// MUST refuse a peer reporting a different major version.
pub const PROTOCOL_MAJOR: u32 = 2;
