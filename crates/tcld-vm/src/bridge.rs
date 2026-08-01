//! HostBridge to embedded Tcl (S05). Gated behind feature `host`.

#![cfg(feature = "host")]

use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::sync::atomic::{AtomicU64, Ordering};

use tcl_sys::{
    TCL_ERROR, TCL_LEAVE_ERR_MSG, TCL_OK, Tcl_CreateObjCommand, Tcl_DecrRefCount,
    Tcl_DeleteCommand, Tcl_DictObjGet, Tcl_EvalObjEx, Tcl_EvalObjv, Tcl_ExprObj,
    Tcl_GetBooleanFromObj, Tcl_GetObjResult, Tcl_GetReturnOptions, Tcl_GetStringFromObj,
    Tcl_IncrRefCount, Tcl_ListObjGetElements, Tcl_NewStringObj, Tcl_Obj, Tcl_ObjGetVar2,
    Tcl_ObjSetVar2, Tcl_ResetResult, Tcl_SetObjResult, Tcl_SetReturnOptions, Tcl_Size,
};
use tcld_interp::{EvalResult, Interpreter};

use crate::error::VmError;
use crate::value::{ByteString, VmObj};

const MAX_REENTER: u32 = 8;
static NEXT_ENGINE_NONCE: AtomicU64 = AtomicU64::new(1);

const OUTPUT_CAPTURE_SHIM: &str = r#"
namespace eval ::tcld {}
if {![llength [info commands ::tcld::_capture_puts_orig]]} {
    rename ::puts ::tcld::_capture_puts_orig
    proc ::puts {args} {
        set original $args
        set nonewline 0
        if {[llength $args] && [lindex $args 0] eq "-nonewline"} {
            set nonewline 1
            set args [lrange $args 1 end]
        }
        if {[llength $args] == 1} {
            set channel stdout
            set text [lindex $args 0]
        } elseif {[llength $args] == 2} {
            set channel [lindex $args 0]
            set text [lindex $args 1]
        } else {
            return [::tcld::_capture_puts_orig {*}$original]
        }
        if {$channel ni {stdout stderr}} {
            return [::tcld::_capture_puts_orig {*}$original]
        }
        append ::tcld::capture_$channel $text
        if {!$nonewline} {
            append ::tcld::capture_$channel "\n"
        }
        return {}
    }
}
set ::tcld::capture_stdout {}
set ::tcld::capture_stderr {}
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeState {
    Idle,
    InHost,
    InVmFromHostStub,
}

/// Result of a host `EvalObjv`.
#[derive(Debug, Clone)]
pub struct BridgeResult {
    pub code: i32,
    pub result: VmObj,
    pub error_info: Option<String>,
    pub error_code: Option<String>,
    pub return_options: Option<VmObj>,
}

#[derive(Debug, Clone)]
pub enum HostFrameStorage {
    Scalar(ByteString),
    Array(Vec<(ByteString, ByteString)>),
    HostLink(ByteString),
}

#[derive(Debug, Clone)]
pub struct HostFrameInput {
    pub name: ByteString,
    pub storage: HostFrameStorage,
}

#[derive(Debug, Clone)]
pub struct HostFrameLocalResult {
    pub name: ByteString,
    pub storage: HostFrameStorage,
}

#[derive(Debug, Clone)]
pub struct HostFrameOutput {
    pub completion: BridgeResult,
    pub locals: Vec<HostFrameLocalResult>,
}

struct FrameEvalContext {
    expression: bool,
    body: ByteString,
    inputs: Vec<HostFrameInput>,
    temp_namespace: ByteString,
    output: Option<Result<HostFrameOutput, VmError>>,
}

thread_local! {
    static ACTIVE_BRIDGE: RefCell<Option<*mut HostBridge>> = const { RefCell::new(None) };
    static STUB_DISPATCH: RefCell<Option<StubDispatchFn>> = const { RefCell::new(None) };
    static ACTIVE_FRAME_EVALS: RefCell<Vec<*mut FrameEvalContext>> = const { RefCell::new(Vec::new()) };
}

type StubDispatchFn = Box<dyn FnMut(&[VmObj]) -> Result<BridgeResult, VmError>>;

/// Opaque client data for CreateObjCommand stubs.
struct StubClient {
    #[allow(dead_code)]
    name: ByteString,
}

pub struct HostBridge {
    pub interp: Interpreter,
    pub state: BridgeState,
    pub reenter_depth: u32,
    /// Stack of states for nested InHost / InVmFromHostStub.
    state_stack: Vec<BridgeState>,
    engine_nonce: u64,
    frame_serial: u64,
}

impl HostBridge {
    pub fn new() -> Result<Self, VmError> {
        let interp = Interpreter::new().map_err(|e| VmError::HybridUnavailable(e.message))?;
        Ok(Self {
            interp,
            state: BridgeState::Idle,
            reenter_depth: 0,
            state_stack: Vec::new(),
            engine_nonce: NEXT_ENGINE_NONCE.fetch_add(1, Ordering::Relaxed),
            frame_serial: 0,
        })
    }

    pub fn install_frame_commands(&self) -> Result<(), VmError> {
        self.install_raw_command(
            "::tcld::frame_eval_internal",
            Some(frame_eval_internal_obj_cmd),
        )?;
        self.install_raw_command(
            "::tcld::frame_active_namespace",
            Some(frame_active_namespace_obj_cmd),
        )
    }

    fn install_raw_command(
        &self,
        name: &str,
        callback: Option<
            unsafe extern "C" fn(
                *mut c_void,
                *mut tcl_sys::Tcl_Interp,
                c_int,
                *const *mut Tcl_Obj,
            ) -> c_int,
        >,
    ) -> Result<(), VmError> {
        let name = CString::new(name)
            .map_err(|_| VmError::Internal("frame command name contains NUL".into()))?;
        unsafe {
            Tcl_CreateObjCommand(
                self.interp.raw(),
                name.as_ptr(),
                callback,
                std::ptr::null_mut(),
                None,
            );
        }
        Ok(())
    }

    pub fn invoke_frame(
        &mut self,
        namespace: &ByteString,
        frame_id: u32,
        expression: bool,
        body: ByteString,
        inputs: Vec<HostFrameInput>,
    ) -> Result<HostFrameOutput, VmError> {
        if self.reenter_depth >= MAX_REENTER {
            return Err(VmError::ReentrancyLimit);
        }
        self.frame_serial = self.frame_serial.wrapping_add(1);
        let serial = self.frame_serial;
        let suffix = format!(
            "__tcld_frame_{}_{}_{}_{}",
            self.engine_nonce, frame_id, self.reenter_depth, serial
        );
        let runner = qualify_bytes(namespace, suffix.as_bytes());
        let temp_namespace = ByteString::from(format!(
            "::tcld::frames::{}::f{}_{}_{}",
            self.engine_nonce, frame_id, self.reenter_depth, serial
        ));

        let namespace_check = self.invoke_objv(&[
            VmObj::from_str("namespace"),
            VmObj::from_str("eval"),
            VmObj::Bytes(namespace.clone()),
            VmObj::empty(),
        ])?;
        if namespace_check.code != TCL_OK {
            return Err(VmError::HostFrameUnavailable(
                namespace_check.result.as_string()?,
            ));
        }
        let temp_check = match self.invoke_objv(&[
            VmObj::from_str("namespace"),
            VmObj::from_str("eval"),
            VmObj::Bytes(temp_namespace.clone()),
            VmObj::empty(),
        ]) {
            Ok(result) => result,
            Err(error) => {
                let _ = self.delete_frame_namespace(&temp_namespace);
                return Err(error);
            }
        };
        if temp_check.code != TCL_OK {
            let _ = self.delete_frame_namespace(&temp_namespace);
            return Err(VmError::HostFrameUnavailable(
                temp_check.result.as_string()?,
            ));
        }
        let collision = match self.invoke_objv(&[
            VmObj::from_str("info"),
            VmObj::from_str("commands"),
            VmObj::Bytes(runner.clone()),
        ]) {
            Ok(result) => result,
            Err(error) => {
                let _ = self.delete_frame_namespace(&temp_namespace);
                return Err(error);
            }
        };
        if collision.code != TCL_OK || !collision.result.ensure_bytes()?.is_empty() {
            let _ = self.delete_frame_namespace(&temp_namespace);
            return Err(VmError::HostFrameUnavailable(format!(
                "host frame command collision: {}",
                runner.as_str_lossy()
            )));
        }
        let create = match self.invoke_objv(&[
            VmObj::from_str("proc"),
            VmObj::Bytes(runner.clone()),
            VmObj::empty(),
            VmObj::from_str("::tcld::frame_eval_internal"),
        ]) {
            Ok(result) => result,
            Err(error) => {
                let _ = self.delete_frame_namespace(&temp_namespace);
                return Err(error);
            }
        };
        if create.code != TCL_OK {
            let _ = self.delete_frame_namespace(&temp_namespace);
            return Err(VmError::HostFrameUnavailable(create.result.as_string()?));
        }

        let mut context = FrameEvalContext {
            expression,
            body,
            inputs,
            temp_namespace: temp_namespace.clone(),
            output: None,
        };
        ACTIVE_FRAME_EVALS.with(|active| {
            active
                .borrow_mut()
                .push(&mut context as *mut FrameEvalContext)
        });
        let invoked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.invoke_objv(&[VmObj::Bytes(runner.clone())])
        }));
        ACTIVE_FRAME_EVALS.with(|active| {
            let popped = active.borrow_mut().pop();
            debug_assert_eq!(popped, Some(&mut context as *mut FrameEvalContext));
        });

        let remove_proc = self.invoke_objv(&[
            VmObj::from_str("rename"),
            VmObj::Bytes(runner),
            VmObj::empty(),
        ]);
        let remove_namespace = self.delete_frame_namespace(&temp_namespace);
        let proc_removed = remove_proc.is_ok_and(|result| result.code == TCL_OK);
        if !proc_removed || remove_namespace.is_err() {
            return Err(VmError::HostFrameUnavailable(
                "failed to clean up ephemeral host frame".into(),
            ));
        }
        match invoked {
            Err(_) => Err(VmError::Internal("panic in host frame bridge".into())),
            Ok(Err(error)) => Err(error),
            Ok(Ok(helper)) if helper.code != TCL_OK => context.output.unwrap_or_else(|| {
                Err(VmError::HostFrameUnavailable(
                    helper
                        .result
                        .as_string()
                        .unwrap_or_else(|_| "typed frame evaluator failed".into()),
                ))
            }),
            Ok(Ok(_)) => context.output.unwrap_or_else(|| {
                Err(VmError::HostFrameUnavailable(
                    "frame evaluator returned no typed result".into(),
                ))
            }),
        }
    }

    fn delete_frame_namespace(&mut self, namespace: &ByteString) -> Result<(), VmError> {
        let result = self.invoke_objv(&[
            VmObj::from_str("namespace"),
            VmObj::from_str("delete"),
            VmObj::Bytes(namespace.clone()),
        ])?;
        if result.code == TCL_OK {
            Ok(())
        } else {
            Err(VmError::HostFrameUnavailable(result.result.as_string()?))
        }
    }

    pub fn find_executable(argv0: Option<&str>) {
        Interpreter::find_executable(argv0);
    }

    pub fn pkg_provide(&self, name: &str, version: &str) -> Result<(), VmError> {
        self.interp
            .pkg_provide(name, version)
            .map_err(|e| VmError::Internal(format!("PkgProvide: {}", e.message)))
    }

    /// Evaluate a host Tcl script (bootstrap / compatibility shims).
    pub fn eval_script(&self, script: &str) -> Result<String, VmError> {
        self.interp.eval(script).map_err(|e| VmError::TclError {
            message: e.message,
            error_info: e.error_info,
            error_code: e.error_code,
        })
    }

    pub fn get_var_bytes(&self, name1: &[u8], name2: Option<&[u8]>) -> Option<Vec<u8>> {
        self.interp.get_var_bytes(name1, name2)
    }

    pub fn set_var_bytes(
        &self,
        name1: &[u8],
        name2: Option<&[u8]>,
        value: &[u8],
    ) -> Result<(), VmError> {
        self.interp
            .set_var_bytes(name1, name2, value)
            .map_err(|e| VmError::TclError {
                message: e.message,
                error_info: e.error_info,
                error_code: e.error_code,
            })
    }

    pub fn begin_output_capture(&self) -> Result<(), VmError> {
        self.eval_script(OUTPUT_CAPTURE_SHIM).map(|_| ())
    }

    pub fn captured_output(&self) -> (ByteString, ByteString) {
        let stdout = self
            .get_var_bytes(b"::tcld::capture_stdout", None)
            .unwrap_or_default();
        let stderr = self
            .get_var_bytes(b"::tcld::capture_stderr", None)
            .unwrap_or_default();
        (
            ByteString::from_bytes(stdout),
            ByteString::from_bytes(stderr),
        )
    }

    /// Install an Option B host obj-command that re-enters via `STUB_DISPATCH`.
    pub fn install_stub_command(&self, name: &str) -> Result<(), VmError> {
        let c_name = CString::new(name)
            .map_err(|_| VmError::Internal(format!("stub name contains NUL: {name}")))?;
        let client = Box::new(StubClient {
            name: ByteString::from(name),
        });
        let client_ptr = Box::into_raw(client) as *mut c_void;
        unsafe {
            let interp = self.interp.raw();
            Tcl_CreateObjCommand(
                interp,
                c_name.as_ptr(),
                Some(stub_obj_cmd),
                client_ptr,
                Some(stub_delete),
            );
        }
        Ok(())
    }

    /// Remove an Option B command owned by the current debug image.
    pub fn uninstall_stub_command(&self, name: &str) -> Result<(), VmError> {
        let c_name = CString::new(name)
            .map_err(|_| VmError::Internal(format!("stub name contains NUL: {name}")))?;
        unsafe {
            // Tcl_DeleteCommand returns -1 only when the command is absent.
            // Absence is harmless while replacing a closed image generation.
            let _ = Tcl_DeleteCommand(self.interp.raw(), c_name.as_ptr());
        }
        Ok(())
    }

    /// Inspect host return options after `TCL_RETURN`.
    ///
    /// Returns `(level, is_plain)`. Plain = level 1 and no non-trivial keys
    /// (`-errorinfo`, `-errorcode`, …). Plan §2.6: non-plain or level>1 →
    /// `CrossRuntimeReturnLevel`.
    pub fn inspect_return_options(&self, result_code: i32) -> Result<(i32, bool), VmError> {
        unsafe {
            let interp = self.interp.raw();
            let opts = Tcl_GetReturnOptions(interp, result_code);
            if opts.is_null() {
                return Ok((1, true));
            }
            // GetReturnOptions returns refcount 1; we own it.
            let key_c = CString::new("-level").expect("static");
            let key_level = Tcl_NewStringObj(key_c.as_ptr(), -1);
            Tcl_IncrRefCount(key_level);
            let mut level_obj: *mut Tcl_Obj = std::ptr::null_mut();
            let _ = Tcl_DictObjGet(interp, opts, key_level, &mut level_obj);
            Tcl_DecrRefCount(key_level);

            let mut level = 1i32;
            if !level_obj.is_null() {
                let mut len: Tcl_Size = 0;
                let ptr = Tcl_GetStringFromObj(level_obj, &mut len);
                if !ptr.is_null() {
                    let s = std::str::from_utf8(std::slice::from_raw_parts(
                        ptr as *const u8,
                        len as usize,
                    ))
                    .unwrap_or("1");
                    level = s.parse().unwrap_or(1);
                }
            }

            // Non-plain return options (plan §2.6): explicit `-options`, or
            // `-code error` / break / continue. Residual `-errorcode` left on
            // the interp after an earlier error is NOT treated as non-plain
            // when `-code` is ok/return/0 (common Tcl GetReturnOptions noise).
            let mut olen: Tcl_Size = 0;
            let optr = Tcl_GetStringFromObj(opts, &mut olen);
            let plain = if optr.is_null() {
                true
            } else {
                let text = std::str::from_utf8(std::slice::from_raw_parts(
                    optr as *const u8,
                    olen as usize,
                ))
                .unwrap_or("");
                let has_options_dict = text.contains("-options");
                let errorish_code = text.contains("-code error")
                    || text.contains("-code break")
                    || text.contains("-code continue")
                    || text.contains("-code 1 ")
                    || text.ends_with("-code 1");
                !(has_options_dict || errorish_code)
            };

            Tcl_DecrRefCount(opts);
            Ok((level, plain))
        }
    }

    /// Access the active bridge during Option B (for nested host invoke).
    pub fn with_active_bridge<R>(f: impl FnOnce(&mut HostBridge) -> R) -> Option<R> {
        ACTIVE_BRIDGE.with(|a| {
            let ptr = *a.borrow();
            ptr.map(|p| unsafe { f(&mut *p) })
        })
    }

    /// Set the stub dispatch closure for Option B re-entry (engine provides this).
    pub fn with_stub_dispatch<R>(
        &mut self,
        dispatch: StubDispatchFn,
        f: impl FnOnce(&mut Self) -> Result<R, VmError>,
    ) -> Result<R, VmError> {
        STUB_DISPATCH.with(|slot| {
            *slot.borrow_mut() = Some(dispatch);
        });
        let self_ptr = self as *mut HostBridge;
        ACTIVE_BRIDGE.with(|a| {
            *a.borrow_mut() = Some(self_ptr);
        });
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(self)));
        ACTIVE_BRIDGE.with(|a| {
            *a.borrow_mut() = None;
        });
        STUB_DISPATCH.with(|slot| {
            *slot.borrow_mut() = None;
        });
        result.unwrap_or_else(|_| Err(VmError::Internal("panic in host bridge callback".into())))
    }

    pub fn invoke_objv(&mut self, words: &[VmObj]) -> Result<BridgeResult, VmError> {
        if words.iter().any(|w| matches!(w, VmObj::ProcBody(_))) {
            return Err(VmError::BridgeMarshalProcBody);
        }
        if self.reenter_depth >= MAX_REENTER {
            return Err(VmError::ReentrancyLimit);
        }

        // Marshal before changing the bridge FSM so nested ProcBody/list
        // failures leave state and depth untouched.
        let mut byte_storage: Vec<ByteString> = Vec::with_capacity(words.len());
        for word in words {
            byte_storage.push(word.ensure_bytes()?);
        }
        let refs = byte_storage
            .iter()
            .map(|bytes| bytes.as_bytes())
            .collect::<Vec<_>>();

        // Allowed: Idle → InHost, or InVmFromHostStub → InHost (nested)
        match self.state {
            BridgeState::Idle | BridgeState::InVmFromHostStub => {}
            BridgeState::InHost => {
                // Nested WorldHost without going through stub is discouraged but
                // depth-accounted re-entry is allowed for nested host calls from VM after stub.
                // Reject free nested InHost→InHost only when depth would exceed — depth check above.
            }
        }

        self.state_stack.push(self.state);
        self.state = BridgeState::InHost;
        self.reenter_depth += 1;

        let evaluated = self
            .interp
            .eval_objv_bytes(&refs)
            .map_err(|error| VmError::Internal(format!("eval_objv: {}", error.message)));

        self.reenter_depth = self.reenter_depth.saturating_sub(1);
        self.state = self.state_stack.pop().unwrap_or(BridgeState::Idle);
        let eval: EvalResult = evaluated?;

        let result = BridgeResult {
            code: eval.code,
            result: VmObj::from_bytes(eval.result),
            error_info: eval
                .error_info
                .map(|b| String::from_utf8_lossy(&b).into_owned()),
            error_code: eval
                .error_code
                .map(|b| String::from_utf8_lossy(&b).into_owned()),
            return_options: eval.return_options.map(VmObj::from_bytes),
        };

        Ok(result)
    }

    /// Enter InVmFromHostStub (Option B).
    pub fn enter_vm_from_stub(&mut self) -> Result<(), VmError> {
        if self.reenter_depth > MAX_REENTER {
            return Err(VmError::ReentrancyLimit);
        }
        self.state_stack.push(self.state);
        self.state = BridgeState::InVmFromHostStub;
        Ok(())
    }

    pub fn leave_vm_from_stub(&mut self) {
        self.state = self.state_stack.pop().unwrap_or(BridgeState::Idle);
    }
}

fn qualify_bytes(namespace: &ByteString, leaf: &[u8]) -> ByteString {
    let mut bytes = namespace.as_bytes().to_vec();
    if bytes != b"::" {
        bytes.extend_from_slice(b"::");
    }
    bytes.extend_from_slice(leaf);
    ByteString::from_bytes(bytes)
}

unsafe extern "C" fn frame_active_namespace_obj_cmd(
    _client_data: *mut c_void,
    interp: *mut tcl_sys::Tcl_Interp,
    _objc: c_int,
    _objv: *const *mut Tcl_Obj,
) -> c_int {
    let namespace = ACTIVE_FRAME_EVALS.with(|active| {
        active
            .borrow()
            .last()
            .map(|context| unsafe { (&**context).temp_namespace.as_bytes().to_vec() })
    });
    let bytes = namespace.unwrap_or_default();
    unsafe {
        Tcl_SetObjResult(interp, new_obj(&bytes));
    }
    TCL_OK
}

unsafe extern "C" fn frame_eval_internal_obj_cmd(
    _client_data: *mut c_void,
    interp: *mut tcl_sys::Tcl_Interp,
    _objc: c_int,
    _objv: *const *mut Tcl_Obj,
) -> c_int {
    let context = ACTIVE_FRAME_EVALS.with(|active| active.borrow().last().copied());
    let Some(context) = context else {
        set_error_result(interp, "frame evaluator invoked without a typed request");
        return TCL_ERROR;
    };
    let evaluated = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        execute_frame_eval(interp, &mut *context)
    }))
    .unwrap_or_else(|_| Err(VmError::Internal("panic in typed frame evaluator".into())));
    unsafe {
        (*context).output = Some(evaluated);
    }
    if unsafe { (*context).output.as_ref().is_some_and(Result::is_ok) } {
        unsafe { Tcl_ResetResult(interp) };
        TCL_OK
    } else {
        let message = unsafe {
            (*context)
                .output
                .as_ref()
                .and_then(|result| result.as_ref().err())
                .map(ToString::to_string)
                .unwrap_or_else(|| "typed frame evaluator failed".into())
        };
        set_error_result(interp, &message);
        TCL_ERROR
    }
}

unsafe fn execute_frame_eval(
    interp: *mut tcl_sys::Tcl_Interp,
    context: &mut FrameEvalContext,
) -> Result<HostFrameOutput, VmError> {
    unsafe {
        for input in &context.inputs {
            match &input.storage {
                HostFrameStorage::Scalar(value) => {
                    set_obj_var(interp, &input.name, None, value)?;
                }
                HostFrameStorage::Array(entries) => {
                    if entries.is_empty() {
                        eval_objv_raw(interp, &[b"array", b"set", input.name.as_bytes(), b""])?;
                    } else {
                        for (key, value) in entries {
                            set_obj_var(interp, &input.name, Some(key), value)?;
                        }
                    }
                }
                HostFrameStorage::HostLink(target) => {
                    eval_objv_raw(
                        interp,
                        &[
                            b"::tcld::_frame_upvar_orig",
                            b"#0",
                            target.as_bytes(),
                            input.name.as_bytes(),
                        ],
                    )?;
                }
            }
        }

        let body = new_obj(context.body.as_bytes());
        Tcl_IncrRefCount(body);
        let mut expression_result = std::ptr::null_mut();
        let code = if context.expression {
            Tcl_ExprObj(interp, body, &mut expression_result)
        } else {
            Tcl_EvalObjEx(interp, body, 0)
        };
        let result_object = if context.expression && code == TCL_OK && !expression_result.is_null()
        {
            expression_result
        } else {
            Tcl_GetObjResult(interp)
        };
        let result = VmObj::from_bytes(obj_bytes(result_object));
        let return_options = if code == TCL_OK {
            None
        } else {
            let options = Tcl_GetReturnOptions(interp, code);
            if options.is_null() {
                None
            } else {
                let value = VmObj::from_bytes(obj_bytes(options));
                Tcl_DecrRefCount(options);
                Some(value)
            }
        };
        Tcl_DecrRefCount(body);
        let completion = BridgeResult {
            code,
            result,
            error_info: None,
            error_code: None,
            return_options,
        };

        eval_objv_raw(interp, &[b"info", b"locals"])?;
        let mut names = context
            .inputs
            .iter()
            .map(|input| input.name.clone())
            .collect::<Vec<_>>();
        for name in list_elements(interp, Tcl_GetObjResult(interp))?
            .into_iter()
            .map(ByteString::from_bytes)
        {
            if !names.contains(&name) {
                names.push(name);
            }
        }
        let mut locals = Vec::with_capacity(names.len());
        for name in names {
            eval_objv_raw(interp, &[b"array", b"exists", name.as_bytes()])?;
            let mut is_array = 0;
            if Tcl_GetBooleanFromObj(interp, Tcl_GetObjResult(interp), &mut is_array) != TCL_OK {
                return Err(interp_error(interp, "array exists returned a non-boolean"));
            }
            if is_array != 0 {
                eval_objv_raw(interp, &[b"array", b"get", name.as_bytes()])?;
                let words = list_elements(interp, Tcl_GetObjResult(interp))?;
                if words.len() % 2 != 0 {
                    return Err(VmError::HostFrameMarshalUnsupported(
                        "array snapshot contains an odd number of words".into(),
                    ));
                }
                let entries = words
                    .chunks_exact(2)
                    .map(|pair| {
                        (
                            ByteString::from_bytes(pair[0].clone()),
                            ByteString::from_bytes(pair[1].clone()),
                        )
                    })
                    .collect();
                locals.push(HostFrameLocalResult {
                    name,
                    storage: HostFrameStorage::Array(entries),
                });
            } else {
                let name_obj = new_obj(name.as_bytes());
                Tcl_IncrRefCount(name_obj);
                let value = Tcl_ObjGetVar2(interp, name_obj, std::ptr::null_mut(), 0);
                if !value.is_null() {
                    locals.push(HostFrameLocalResult {
                        name,
                        storage: HostFrameStorage::Scalar(ByteString::from_bytes(obj_bytes(value))),
                    });
                }
                Tcl_DecrRefCount(name_obj);
            }
        }
        Ok(HostFrameOutput { completion, locals })
    }
}

unsafe fn set_obj_var(
    interp: *mut tcl_sys::Tcl_Interp,
    name1: &ByteString,
    name2: Option<&ByteString>,
    value: &ByteString,
) -> Result<(), VmError> {
    unsafe {
        let part1 = new_obj(name1.as_bytes());
        Tcl_IncrRefCount(part1);
        let part2 = name2.map(|name| {
            let obj = new_obj(name.as_bytes());
            Tcl_IncrRefCount(obj);
            obj
        });
        let value_obj = new_obj(value.as_bytes());
        let stored = Tcl_ObjSetVar2(
            interp,
            part1,
            part2.unwrap_or(std::ptr::null_mut()),
            value_obj,
            TCL_LEAVE_ERR_MSG,
        );
        if let Some(part2) = part2 {
            Tcl_DecrRefCount(part2);
        }
        Tcl_DecrRefCount(part1);
        if stored.is_null() {
            Err(interp_error(interp, "failed to seed frame variable"))
        } else {
            Ok(())
        }
    }
}

unsafe fn eval_objv_raw(interp: *mut tcl_sys::Tcl_Interp, words: &[&[u8]]) -> Result<(), VmError> {
    unsafe {
        let objects = words
            .iter()
            .map(|word| {
                let obj = new_obj(word);
                Tcl_IncrRefCount(obj);
                obj
            })
            .collect::<Vec<_>>();
        let code = Tcl_EvalObjv(interp, objects.len() as c_int, objects.as_ptr(), 0);
        for object in objects {
            Tcl_DecrRefCount(object);
        }
        if code == TCL_OK {
            Ok(())
        } else {
            Err(interp_error(interp, "host frame helper command failed"))
        }
    }
}

unsafe fn list_elements(
    interp: *mut tcl_sys::Tcl_Interp,
    list: *mut Tcl_Obj,
) -> Result<Vec<Vec<u8>>, VmError> {
    unsafe {
        let mut count: Tcl_Size = 0;
        let mut elements: *mut *mut Tcl_Obj = std::ptr::null_mut();
        if Tcl_ListObjGetElements(interp, list, &mut count, &mut elements) != TCL_OK {
            return Err(interp_error(
                interp,
                "host frame helper returned an invalid list",
            ));
        }
        Ok((0..count as usize)
            .map(|index| obj_bytes(*elements.add(index)))
            .collect())
    }
}

unsafe fn new_obj(bytes: &[u8]) -> *mut Tcl_Obj {
    unsafe { Tcl_NewStringObj(bytes.as_ptr() as *const c_char, bytes.len() as Tcl_Size) }
}

unsafe fn obj_bytes(obj: *mut Tcl_Obj) -> Vec<u8> {
    unsafe {
        if obj.is_null() {
            return Vec::new();
        }
        let mut length: Tcl_Size = 0;
        let bytes = Tcl_GetStringFromObj(obj, &mut length);
        if bytes.is_null() {
            Vec::new()
        } else {
            std::slice::from_raw_parts(bytes as *const u8, length as usize).to_vec()
        }
    }
}

fn interp_error(interp: *mut tcl_sys::Tcl_Interp, fallback: &str) -> VmError {
    let message = unsafe { obj_bytes(Tcl_GetObjResult(interp)) };
    VmError::HostFrameMarshalUnsupported(if message.is_empty() {
        fallback.into()
    } else {
        String::from_utf8_lossy(&message).into_owned()
    })
}

fn set_error_result(interp: *mut tcl_sys::Tcl_Interp, message: &str) {
    unsafe {
        Tcl_SetObjResult(interp, new_obj(message.as_bytes()));
    }
}

unsafe extern "C" fn stub_obj_cmd(
    client_data: *mut c_void,
    interp: *mut tcl_sys::Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    if client_data.is_null() || objv.is_null() {
        return TCL_ERROR;
    }
    let mut words: Vec<VmObj> = Vec::with_capacity(objc as usize);
    for i in 0..objc as isize {
        let obj = unsafe { *objv.offset(i) };
        if obj.is_null() {
            words.push(VmObj::empty());
            continue;
        }
        let mut len: Tcl_Size = 0;
        let ptr = unsafe { Tcl_GetStringFromObj(obj, &mut len) };
        if ptr.is_null() {
            words.push(VmObj::empty());
        } else {
            let slice = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
            words.push(VmObj::from_bytes(slice.to_vec()));
        }
    }

    let result = STUB_DISPATCH.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some(dispatch) = slot.as_mut() {
            dispatch(&words)
        } else {
            Err(VmError::Internal(
                "stub invoked without active STUB_DISPATCH".into(),
            ))
        }
    });

    match result {
        Ok(br) => {
            let bytes = match br.result.ensure_bytes() {
                Ok(bytes) => bytes,
                Err(error) => {
                    set_error_result(interp, &error.to_string());
                    return TCL_ERROR;
                }
            };
            unsafe {
                let obj = Tcl_NewStringObj(
                    bytes.as_bytes().as_ptr() as *const c_char,
                    bytes.as_bytes().len() as Tcl_Size,
                );
                Tcl_SetObjResult(interp, obj);
            }
            if let Some(options) = br.return_options {
                let options = match options.ensure_bytes() {
                    Ok(options) => options,
                    Err(error) => {
                        set_error_result(interp, &error.to_string());
                        return TCL_ERROR;
                    }
                };
                unsafe {
                    let object = new_obj(options.as_bytes());
                    Tcl_IncrRefCount(object);
                    let code = Tcl_SetReturnOptions(interp, object);
                    Tcl_DecrRefCount(object);
                    code
                }
            } else {
                br.code
            }
        }
        Err(e) => {
            let msg = e.to_string();
            unsafe {
                let obj = Tcl_NewStringObj(msg.as_ptr() as *const c_char, msg.len() as Tcl_Size);
                Tcl_SetObjResult(interp, obj);
            }
            TCL_ERROR
        }
    }
}

unsafe extern "C" fn stub_delete(client_data: *mut c_void) {
    if !client_data.is_null() {
        drop(unsafe { Box::from_raw(client_data as *mut StubClient) });
    }
}

// Silence unused import warnings for CStr if not used.
#[allow(dead_code)]
fn _use_cstr(_: &CStr) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::completion::ReturnOptions;

    #[test]
    fn objv_preserves_raw_return_options() {
        let mut bridge = HostBridge::new().expect("embedded Tcl");
        let result = bridge
            .invoke_objv(&[VmObj::from_str("error"), VmObj::from_str("custom")])
            .expect("error invocation");
        assert_eq!(result.code, tcl_sys::TCL_ERROR);
        assert_eq!(result.result, VmObj::from_str("custom"));
        let options =
            ReturnOptions::from_vm_obj(result.return_options.expect("raw return options"))
                .expect("parse return options");
        assert_eq!(options.level, 0);
        assert_eq!(options.code, 1);
        assert!(options.error_info.is_some());
    }
}
