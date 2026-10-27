#![allow(non_camel_case_types)]

use std::os::raw::{c_char, c_int, c_void};

#[cfg(feature = "tcl9")]
pub type Tcl_Size = isize;
#[cfg(not(feature = "tcl9"))]
pub type Tcl_Size = c_int;

// Opaque types
pub enum Tcl_Interp {}
pub enum Tcl_Obj {}
pub enum Tcl_Trace {}
pub enum Tcl_ObjType {}
pub enum Tcl_Command_ {}
pub type Tcl_Command = *mut Tcl_Command_;

// Return codes
pub const TCL_OK: c_int = 0;
pub const TCL_ERROR: c_int = 1;
pub const TCL_RETURN: c_int = 2;
pub const TCL_BREAK: c_int = 3;
pub const TCL_CONTINUE: c_int = 4;

// Variable flags
pub const TCL_GLOBAL_ONLY: c_int = 1;
pub const TCL_LEAVE_ERR_MSG: c_int = 0x200;
pub const TCL_NAMESPACE_ONLY: c_int = 0x02;

// Event flags
pub const TCL_DONT_WAIT: c_int = 1 << 1;
pub const TCL_SERVICE_ALL: c_int = 1;

pub type Tcl_CmdObjTraceProc = unsafe extern "C" fn(
    client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    level: c_int,
    command: *const c_char,
    command_token: Tcl_Command,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int;

pub type Tcl_CmdObjTraceDeleteProc = unsafe extern "C" fn(client_data: *mut c_void);

#[repr(C)]
pub struct Tcl_CmdInfo {
    pub is_native_object_proc: c_int,
    pub obj_proc: Option<
        unsafe extern "C" fn(
            client_data: *mut c_void,
            interp: *mut Tcl_Interp,
            objc: c_int,
            objv: *const *mut Tcl_Obj,
        ) -> c_int,
    >,
    pub obj_client_data: *mut c_void,
    pub proc_: Option<
        unsafe extern "C" fn(
            client_data: *mut c_void,
            interp: *mut Tcl_Interp,
            argc: c_int,
            argv: *const *const c_char,
        ) -> c_int,
    >,
    pub client_data: *mut c_void,
    pub delete_proc: Option<unsafe extern "C" fn(client_data: *mut c_void)>,
    pub delete_data: *mut c_void,
    pub namespace_ptr: *mut c_void,
}

unsafe extern "C" {
    // Lifecycle
    pub fn Tcl_CreateInterp() -> *mut Tcl_Interp;
    pub fn Tcl_DeleteInterp(interp: *mut Tcl_Interp);
    pub fn Tcl_Init(interp: *mut Tcl_Interp) -> c_int;
    /// Initializes per-thread Tcl runtime state including the notifier.
    /// Required before any use of the event loop (`vwait`, `after`,
    /// fileevent). Pass `NULL` when the embedding application has no
    /// own argv[0] to register.
    #[cfg(feature = "tcl9")]
    pub fn Tcl_FindExecutable(arg0: *const c_char) -> *const c_char;
    #[cfg(not(feature = "tcl9"))]
    pub fn Tcl_FindExecutable(arg0: *const c_char);
    pub fn Tcl_InitStubs(
        interp: *mut Tcl_Interp,
        version: *const c_char,
        exact: c_int,
    ) -> *const c_char;
    pub fn Tcl_DoOneEvent(flags: c_int) -> c_int;
    pub fn Tcl_SetServiceMode(mode: c_int) -> c_int;

    // Evaluation
    pub fn Tcl_EvalFile(interp: *mut Tcl_Interp, file: *const c_char) -> c_int;
    pub fn Tcl_EvalEx(
        interp: *mut Tcl_Interp,
        script: *const c_char,
        num_bytes: Tcl_Size,
        flags: c_int,
    ) -> c_int;
    pub fn Tcl_EvalObjv(
        interp: *mut Tcl_Interp,
        objc: c_int,
        objv: *const *mut Tcl_Obj,
        flags: c_int,
    ) -> c_int;
    pub fn Tcl_EvalObjEx(interp: *mut Tcl_Interp, obj: *mut Tcl_Obj, flags: c_int) -> c_int;
    pub fn Tcl_ExprObj(
        interp: *mut Tcl_Interp,
        obj: *mut Tcl_Obj,
        result_ptr: *mut *mut Tcl_Obj,
    ) -> c_int;
    // NOTE: Tcl_GetStringResult is deliberately not declared — Tcl 9 does
    // not export it. Read results via Tcl_GetObjResult + Tcl_GetStringFromObj.
    pub fn Tcl_GetObjResult(interp: *mut Tcl_Interp) -> *mut Tcl_Obj;
    pub fn Tcl_ResetResult(interp: *mut Tcl_Interp);

    // Tracing
    pub fn Tcl_CreateObjTrace(
        interp: *mut Tcl_Interp,
        level: Tcl_Size,
        flags: c_int,
        obj_proc: Tcl_CmdObjTraceProc,
        client_data: *mut c_void,
        delete_proc: Option<Tcl_CmdObjTraceDeleteProc>,
    ) -> *mut Tcl_Trace;
    pub fn Tcl_DeleteTrace(interp: *mut Tcl_Interp, trace: *mut Tcl_Trace);

    // Variables
    pub fn Tcl_GetVar2Ex(
        interp: *mut Tcl_Interp,
        name1: *const c_char,
        name2: *const c_char,
        flags: c_int,
    ) -> *mut Tcl_Obj;
    pub fn Tcl_SetVar2Ex(
        interp: *mut Tcl_Interp,
        name1: *const c_char,
        name2: *const c_char,
        new_value: *mut Tcl_Obj,
        flags: c_int,
    ) -> *mut Tcl_Obj;

    // Objects
    pub fn Tcl_GetStringFromObj(obj: *mut Tcl_Obj, length: *mut Tcl_Size) -> *const c_char;
    pub fn Tcl_NewStringObj(bytes: *const c_char, length: Tcl_Size) -> *mut Tcl_Obj;
    pub fn Tcl_SetObjResult(interp: *mut Tcl_Interp, result_obj_ptr: *mut Tcl_Obj);
    pub fn Tcl_IncrRefCount(obj: *mut Tcl_Obj);
    pub fn Tcl_DecrRefCount(obj: *mut Tcl_Obj);

    // Pkg require
    pub fn Tcl_PkgRequire(
        interp: *mut Tcl_Interp,
        name: *const c_char,
        version: *const c_char,
        exact: c_int,
    ) -> *const c_char;
    pub fn Tcl_PkgProvide(
        interp: *mut Tcl_Interp,
        name: *const c_char,
        version: *const c_char,
    ) -> c_int;

    // Commands
    pub fn Tcl_CreateObjCommand(
        interp: *mut Tcl_Interp,
        cmd_name: *const c_char,
        proc: Option<
            unsafe extern "C" fn(
                client_data: *mut c_void,
                interp: *mut Tcl_Interp,
                objc: c_int,
                objv: *const *mut Tcl_Obj,
            ) -> c_int,
        >,
        client_data: *mut c_void,
        delete_proc: Option<unsafe extern "C" fn(client_data: *mut c_void)>,
    ) -> *mut c_void;
    pub fn Tcl_DeleteCommand(interp: *mut Tcl_Interp, cmd_name: *const c_char) -> c_int;

    pub fn Tcl_FindCommand(
        interp: *mut Tcl_Interp,
        name: *const c_char,
        context_ns: *mut c_void,
        flags: c_int,
    ) -> Tcl_Command;

    pub fn Tcl_GetCommandFullName(
        interp: *mut Tcl_Interp,
        command: Tcl_Command,
        obj_ptr: *mut Tcl_Obj,
    );

    pub fn Tcl_GetCommandInfoFromToken(command: Tcl_Command, info_ptr: *mut Tcl_CmdInfo) -> c_int;

    pub fn Tcl_FindNamespace(
        interp: *mut Tcl_Interp,
        name: *const c_char,
        context_ns: *mut c_void,
        flags: c_int,
    ) -> *mut c_void;

    // Thread
    pub fn Tcl_GetCurrentThread() -> *mut c_void;
    /// Tears down this thread's Tcl state. Every thread that created an
    /// interp MUST call this (after deleting its interps) before it
    /// exits. Tcl keys per-thread notifier records by thread id in a
    /// process-global list; a thread that exits without finalizing
    /// leaves a zombie entry there, and when the OS hands the same
    /// thread id to a later thread (glibc reuses a joined pthread_t
    /// deterministically) Tcl mistakes it for the dead thread and skips
    /// platform notifier init — the next `fileevent` registration then
    /// aborts the process from inside Tcl (`epoll_ctl: Invalid
    /// argument` on Linux).
    pub fn Tcl_FinalizeThread();

    // Object introspection — Tcl 9 only. 8.6 doesn't expose
    // Tcl_HasStringRep / Tcl_FetchInternalRep as public C symbols, so
    // these bindings (and any callers) are gated. Use the version-
    // agnostic `has_string_rep` / `fetch_internal_rep` helpers below.
    #[cfg(feature = "tcl9")]
    pub fn Tcl_HasStringRep(obj: *mut Tcl_Obj) -> c_int;
    #[cfg(feature = "tcl9")]
    pub fn Tcl_FetchInternalRep(obj: *mut Tcl_Obj, type_ptr: *const Tcl_ObjType) -> *mut c_void;
    pub fn Tcl_GetObjType(name: *const c_char) -> *const Tcl_ObjType;
}

/// Tcl 9 exposes `Tcl_HasStringRep`; 8.6 does not. The 8.6 fallback
/// is conservative ("treat every obj as having a string rep") because
/// no callable replacement exists in the public 8.6 API.
///
/// # Safety
///
/// `obj` must be a valid `Tcl_Obj` pointer.
#[cfg(feature = "tcl9")]
pub unsafe fn has_string_rep(obj: *mut Tcl_Obj) -> bool {
    unsafe { Tcl_HasStringRep(obj) != 0 }
}

#[cfg(not(feature = "tcl9"))]
pub unsafe fn has_string_rep(_obj: *mut Tcl_Obj) -> bool {
    true
}

/// Tcl 9 exposes `Tcl_FetchInternalRep`; 8.6 does not. The 8.6
/// fallback returns NULL — callers that depend on the internal rep
/// for behaviour MUST be feature-gated to 9.
///
/// # Safety
///
/// `obj` and `type_ptr` must be valid pointers when non-null.
#[cfg(feature = "tcl9")]
pub unsafe fn fetch_internal_rep(obj: *mut Tcl_Obj, type_ptr: *const Tcl_ObjType) -> *mut c_void {
    unsafe { Tcl_FetchInternalRep(obj, type_ptr) }
}

#[cfg(not(feature = "tcl9"))]
pub unsafe fn fetch_internal_rep(_obj: *mut Tcl_Obj, _type_ptr: *const Tcl_ObjType) -> *mut c_void {
    std::ptr::null_mut()
}

unsafe extern "C" {

    // Type-safe accessors
    pub fn Tcl_ListObjLength(
        interp: *mut Tcl_Interp,
        list_obj: *mut Tcl_Obj,
        int_ptr: *mut Tcl_Size,
    ) -> c_int;
    pub fn Tcl_DictObjSize(
        interp: *mut Tcl_Interp,
        dict_obj: *mut Tcl_Obj,
        size_ptr: *mut Tcl_Size,
    ) -> c_int;
    /// Retrieve return options after a command returned `TCL_RETURN` (or other
    /// non-OK code). Returned object has refcount 1 — caller must DecrRefCount.
    pub fn Tcl_GetReturnOptions(interp: *mut Tcl_Interp, result: c_int) -> *mut Tcl_Obj;
    pub fn Tcl_SetReturnOptions(interp: *mut Tcl_Interp, options: *mut Tcl_Obj) -> c_int;
    pub fn Tcl_DictObjGet(
        interp: *mut Tcl_Interp,
        dict_ptr: *mut Tcl_Obj,
        key_ptr: *mut Tcl_Obj,
        value_ptr_ptr: *mut *mut Tcl_Obj,
    ) -> c_int;
    pub fn Tcl_GetIntFromObj(
        interp: *mut Tcl_Interp,
        obj: *mut Tcl_Obj,
        int_ptr: *mut c_int,
    ) -> c_int;
    pub fn Tcl_GetDoubleFromObj(
        interp: *mut Tcl_Interp,
        obj: *mut Tcl_Obj,
        double_ptr: *mut f64,
    ) -> c_int;
    pub fn Tcl_GetBooleanFromObj(
        interp: *mut Tcl_Interp,
        obj: *mut Tcl_Obj,
        bool_ptr: *mut c_int,
    ) -> c_int;
    pub fn Tcl_ListObjGetElements(
        interp: *mut Tcl_Interp,
        list_obj: *mut Tcl_Obj,
        objc_ptr: *mut Tcl_Size,
        objv_ptr: *mut *mut *mut Tcl_Obj,
    ) -> c_int;
    pub fn Tcl_ObjGetVar2(
        interp: *mut Tcl_Interp,
        part1: *mut Tcl_Obj,
        part2: *mut Tcl_Obj,
        flags: c_int,
    ) -> *mut Tcl_Obj;
    pub fn Tcl_ObjSetVar2(
        interp: *mut Tcl_Interp,
        part1: *mut Tcl_Obj,
        part2: *mut Tcl_Obj,
        new_value: *mut Tcl_Obj,
        flags: c_int,
    ) -> *mut Tcl_Obj;

    /// Look up a child (formerly "slave") interpreter by name. The
    /// child name is the Tcl-visible name passed to `interp create`.
    /// Returns NULL if no such child exists.
    ///
    /// Tcl 9 renamed `Tcl_GetSlave` to `Tcl_GetChild`. We bind the
    /// version-appropriate symbol via cfg and expose a single Rust
    /// function `Tcl_GetChild` for callers.
    #[cfg(feature = "tcl9")]
    pub fn Tcl_GetChild(interp: *mut Tcl_Interp, child_name: *const c_char) -> *mut Tcl_Interp;

    #[cfg(not(feature = "tcl9"))]
    #[link_name = "Tcl_GetSlave"]
    pub fn Tcl_GetChild(interp: *mut Tcl_Interp, child_name: *const c_char) -> *mut Tcl_Interp;
}
