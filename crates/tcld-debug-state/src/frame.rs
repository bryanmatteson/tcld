//! Stack frame snapshot.
//!
//! TclPro's [`dbg::getStack`](https://github.com/.../dbg.tcl) returns a list
//! of `{level type location args}`. [`FrameSnapshot`] is the same
//! payload, plus the proc qname when applicable. The trace handler
//! builds these from `info level` + `info frame` whenever the host calls
//! `inspect_stack` while paused.

use crate::location::Location;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrameKind {
    /// Top level of the interpreter.
    Toplevel,
    /// Inside a `proc` body.
    Proc,
    /// Inside a `namespace eval` body.
    NamespaceEval,
    /// Inside an `eval` / `apply` / `uplevel` body.
    Eval,
    /// Inside a body loaded via `source` (file-scope script
    /// execution). Tcl distinguishes this from generic `eval` because
    /// it carries reliable `file` and `line` fields.
    File,
    /// Tcl `info frame` reports `type=precompiled` for a body whose
    /// origin source was discarded (e.g. tbcload-loaded bcprocs that
    /// the debugger hasn't re-instrumented). No `line` info is
    /// available; the host should not expect a `Location`.
    Precompiled,
    /// Tcl reported a frame type we don't have a mapping for.
    Other,
}

impl FrameKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Toplevel => "toplevel",
            Self::Proc => "proc",
            Self::NamespaceEval => "namespace_eval",
            Self::Eval => "eval",
            Self::File => "file",
            Self::Precompiled => "precompiled",
            Self::Other => "other",
        }
    }

    /// Map a Tcl `info frame` `type` field to a [`FrameKind`]. Tcl 9
    /// uses these strings:
    ///
    /// - `"source"` — script loaded from a file; `file` and `line`
    ///   fields are reliable
    /// - `"proc"` — currently executing a proc body
    /// - `"eval"` — inside an `eval`/`apply`/`uplevel` script
    ///   argument; no `file` info
    /// - `"precompiled"` — body whose source was lost (tbcload, etc.)
    ///
    /// Empty string and `"toplevel"` map to `Toplevel`.
    pub fn from_tcl_frame_type(s: &str) -> Self {
        match s {
            "" | "toplevel" => Self::Toplevel,
            "proc" => Self::Proc,
            "source" => Self::File,
            "eval" => Self::Eval,
            "namespace eval" | "namespace_eval" => Self::NamespaceEval,
            "precompiled" => Self::Precompiled,
            _ => Self::Other,
        }
    }
}

/// One stack frame's worth of debug-time information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameSnapshot {
    /// Tcl level depth. 0 == top level, 1 == first proc call, etc.
    pub level: u32,
    pub kind: FrameKind,
    /// Qualified proc name when `kind == Proc`. None otherwise.
    pub proc_qname: Option<Arc<str>>,
    /// Source file reported by Tcl, or the active StartTarget file for a
    /// proc body whose frame dictionary omits file provenance.
    pub source_path: Option<Arc<str>>,
    /// Location of the currently-executing command in this frame, when
    /// derivable. None for top-level or unknown frames.
    pub location: Option<Location>,
    /// Words of the call as Tcl saw them (`info level $level`). For
    /// `kind == Proc` this is the call expression that entered the
    /// frame; for top-level this is empty.
    pub args: Vec<String>,
    /// When this frame is currently executing a script that was
    /// dispatched to it via `uplevel` from another frame, the level
    /// number of the frame that originated the uplevel call. None for
    /// frames that aren't currently servicing an uplevel'd body. Lets
    /// the debugger UI render "stmt's source originated in frame X"
    /// instead of pretending the executing frame is the only context.
    pub via_uplevel_from: Option<u32>,
}

/// Records one in-progress uplevel transition. Pushed when an `uplevel`
/// wrapper enters; popped when it exits. Held on
/// [`crate::DebugState::uplevel_hops`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UplevelHop {
    /// Tcl level that issued the `uplevel` call.
    pub from_level: u32,
    /// Tcl level the uplevel'd body is executing in. The destination.
    pub to_level: u32,
}
