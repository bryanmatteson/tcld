use std::ffi::{CStr, CString};
use std::marker::PhantomData;
use std::os::raw::{c_char, c_int, c_void};

use tcl_sys::*;

// Tcl keeps a process-wide notifier registry keyed by OS thread ID. Release
// it when a Rust-owned thread exits so a reused thread ID is initialized anew.
struct TclThreadGuard;
impl Drop for TclThreadGuard {
    fn drop(&mut self) {
        unsafe {
            Tcl_FinalizeThread();
        }
    }
}
thread_local! {static TCL_THREAD: TclThreadGuard = const {TclThreadGuard};}

type TraceCallback = Box<dyn FnMut(&str, i32) -> bool>;

/// Error returned by Tcl operations.
#[derive(Debug, Clone)]
pub struct TclError {
    pub message: String,
    pub error_info: Option<String>,
    pub error_code: Option<String>,
}

impl std::fmt::Display for TclError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for TclError {}

/// A safe wrapper around a Tcl interpreter.
///
/// This type is intentionally `!Send` and `!Sync` — a Tcl interpreter
/// must only be used from the thread that created it.
pub struct Interpreter {
    interp: *mut Tcl_Interp,
    trace: Option<*mut Tcl_Trace>,
    _not_send: PhantomData<*mut ()>,
}

impl Interpreter {
    /// Create a new Tcl interpreter and call `Tcl_Init`.
    pub fn new() -> Result<Self, TclError> {
        TCL_THREAD.with(|_| {});
        unsafe {
            let interp = Tcl_CreateInterp();
            if interp.is_null() {
                return Err(TclError {
                    message: "Tcl_CreateInterp returned null".into(),
                    error_info: None,
                    error_code: None,
                });
            }
            let ret = Tcl_Init(interp);
            if ret != TCL_OK {
                let msg = get_string_result(interp);
                Tcl_DeleteInterp(interp);
                return Err(TclError {
                    message: format!("Tcl_Init failed: {msg}"),
                    error_info: None,
                    error_code: None,
                });
            }
            Ok(Interpreter {
                interp,
                trace: None,
                _not_send: PhantomData,
            })
        }
    }

    /// Evaluate a Tcl script string. Returns the result string on success.
    pub fn eval(&self, script: &str) -> Result<String, TclError> {
        let c_script = CString::new(script).map_err(|e| TclError {
            message: format!("script contains null byte: {e}"),
            error_info: None,
            error_code: None,
        })?;
        unsafe {
            let ret = Tcl_EvalEx(self.interp, c_script.as_ptr(), -1, 0);
            let result = get_string_result(self.interp);
            if ret == TCL_OK {
                Ok(result)
            } else {
                Err(self.make_error(result))
            }
        }
    }

    /// Evaluate a file (Tcl_EvalFile).
    pub fn eval_file(&self, path: &str) -> Result<String, TclError> {
        let c_path = CString::new(path).map_err(|e| TclError {
            message: format!("path contains null byte: {e}"),
            error_info: None,
            error_code: None,
        })?;
        unsafe {
            let ret = Tcl_EvalFile(self.interp, c_path.as_ptr());
            let result = get_string_result(self.interp);
            if ret == TCL_OK {
                Ok(result)
            } else {
                Err(self.make_error(result))
            }
        }
    }

    /// Get a variable value. `name` is the variable name.
    /// Returns None if the variable doesn't exist.
    pub fn get_var(&self, name: &str) -> Option<String> {
        let c_name = CString::new(name).ok()?;
        unsafe {
            let obj = Tcl_GetVar2Ex(self.interp, c_name.as_ptr(), std::ptr::null(), 0);
            if obj.is_null() {
                None
            } else {
                Some(obj_to_string(obj))
            }
        }
    }

    /// Set a variable value.
    pub fn set_var(&self, name: &str, value: &str) -> Result<(), TclError> {
        let c_name = CString::new(name).map_err(|_| TclError {
            message: "name contains null byte".into(),
            error_info: None,
            error_code: None,
        })?;
        let c_value = CString::new(value).map_err(|_| TclError {
            message: "value contains null byte".into(),
            error_info: None,
            error_code: None,
        })?;
        unsafe {
            let new_obj = Tcl_NewStringObj(c_value.as_ptr(), -1);
            let result = Tcl_SetVar2Ex(
                self.interp,
                c_name.as_ptr(),
                std::ptr::null(),
                new_obj,
                TCL_LEAVE_ERR_MSG,
            );
            if result.is_null() {
                Err(self.make_error(get_string_result(self.interp)))
            } else {
                Ok(())
            }
        }
    }

    /// Install a command trace. The callback fires before each command is dispatched.
    ///
    /// The callback receives:
    /// - `command`: the command string about to execute
    /// - `level`: the stack level
    ///
    /// It returns `true` to continue execution or `false` to abort (returns TCL_ERROR).
    ///
    /// Only one trace can be active at a time. Setting a new trace replaces the old one.
    pub fn set_trace<F>(&mut self, callback: F)
    where
        F: FnMut(&str, i32) -> bool + 'static,
    {
        self.remove_trace();

        let boxed: Box<TraceCallback> = Box::new(Box::new(callback));
        let client_data = Box::into_raw(boxed) as *mut c_void;

        unsafe {
            let trace = Tcl_CreateObjTrace(
                self.interp,
                0, // level 0 = all levels
                0, // flags
                trace_callback_trampoline,
                client_data,
                Some(trace_delete_callback),
            );
            self.trace = Some(trace);
        }
    }

    /// Remove the current command trace, if any.
    pub fn remove_trace(&mut self) {
        if let Some(trace) = self.trace.take() {
            unsafe {
                Tcl_DeleteTrace(self.interp, trace);
            }
        }
    }

    /// Get the raw interp pointer (for advanced use).
    ///
    /// # Safety
    /// The caller must not delete the interp or use it after the `Interpreter` is dropped.
    pub unsafe fn raw(&self) -> *mut Tcl_Interp {
        self.interp
    }

    /// Bootstrap Tcl library paths (`Tcl_FindExecutable`).
    pub fn find_executable(argv0: Option<&str>) {
        unsafe {
            match argv0 {
                Some(s) => {
                    if let Ok(c) = CString::new(s) {
                        Tcl_FindExecutable(c.as_ptr());
                    } else {
                        Tcl_FindExecutable(std::ptr::null());
                    }
                }
                None => {
                    Tcl_FindExecutable(std::ptr::null());
                }
            }
        }
    }

    /// `Tcl_PkgProvide` — e.g. claim `tbcload` is already present.
    pub fn pkg_provide(&self, name: &str, version: &str) -> Result<(), TclError> {
        let c_name = CString::new(name).map_err(|_| TclError {
            message: "name contains null byte".into(),
            error_info: None,
            error_code: None,
        })?;
        let c_ver = CString::new(version).map_err(|_| TclError {
            message: "version contains null byte".into(),
            error_info: None,
            error_code: None,
        })?;
        unsafe {
            let rc = Tcl_PkgProvide(self.interp, c_name.as_ptr(), c_ver.as_ptr());
            if rc != TCL_OK {
                Err(self.make_error(get_string_result(self.interp)))
            } else {
                Ok(())
            }
        }
    }

    /// Length-aware object-vector eval (hybrid HostBridge hot path).
    ///
    /// Words may contain interior NUL bytes.
    pub fn eval_objv_bytes(&self, words: &[&[u8]]) -> Result<EvalResult, TclError> {
        if words.is_empty() {
            return Ok(EvalResult {
                code: TCL_OK,
                result: Vec::new(),
                error_info: None,
                error_code: None,
                return_options: None,
            });
        }
        unsafe {
            let mut objs: Vec<*mut Tcl_Obj> = Vec::with_capacity(words.len());
            for w in words {
                let obj = Tcl_NewStringObj(w.as_ptr() as *const c_char, w.len() as Tcl_Size);
                Tcl_IncrRefCount(obj);
                objs.push(obj);
            }
            let code = Tcl_EvalObjv(self.interp, objs.len() as c_int, objs.as_ptr(), 0);
            let result = obj_to_bytes(Tcl_GetObjResult(self.interp));
            let return_options = if code == TCL_OK {
                None
            } else {
                let options = Tcl_GetReturnOptions(self.interp, code);
                if options.is_null() {
                    None
                } else {
                    let bytes = obj_to_bytes(options);
                    Tcl_DecrRefCount(options);
                    Some(bytes)
                }
            };
            let err = if code != TCL_OK {
                Some(self.make_error(String::from_utf8_lossy(&result).into_owned()))
            } else {
                None
            };
            for obj in objs {
                Tcl_DecrRefCount(obj);
            }
            if let Some(e) = err {
                Ok(EvalResult {
                    code,
                    result,
                    error_info: e.error_info.map(|s| s.into_bytes()),
                    error_code: e.error_code.map(|s| s.into_bytes()),
                    return_options,
                })
            } else {
                Ok(EvalResult {
                    code,
                    result,
                    error_info: None,
                    error_code: None,
                    return_options,
                })
            }
        }
    }

    /// Parse a counted-byte Tcl list with Tcl's own object parser.
    pub fn split_list_bytes(&self, list: &[u8]) -> Result<Vec<Vec<u8>>, TclError> {
        unsafe {
            let obj = Tcl_NewStringObj(list.as_ptr() as *const c_char, list.len() as Tcl_Size);
            Tcl_IncrRefCount(obj);
            let mut objc: Tcl_Size = 0;
            let mut objv: *mut *mut Tcl_Obj = std::ptr::null_mut();
            let code = Tcl_ListObjGetElements(self.interp, obj, &mut objc, &mut objv);
            if code != TCL_OK {
                let error = self.make_error(get_string_result(self.interp));
                Tcl_DecrRefCount(obj);
                return Err(error);
            }
            let mut elements = Vec::with_capacity(objc as usize);
            for index in 0..objc as usize {
                elements.push(obj_to_bytes(*objv.add(index)));
            }
            Tcl_DecrRefCount(obj);
            Ok(elements)
        }
    }

    /// Length-aware variable get (name may contain interior NUL in theory; we use CString for Tcl API name path).
    pub fn get_var_bytes(&self, name1: &[u8], name2: Option<&[u8]>) -> Option<Vec<u8>> {
        let c1 = CString::new(name1).ok()?;
        let c2 = name2.and_then(|n| CString::new(n).ok());
        unsafe {
            let obj = Tcl_GetVar2Ex(
                self.interp,
                c1.as_ptr(),
                c2.as_ref().map(|c| c.as_ptr()).unwrap_or(std::ptr::null()),
                0,
            );
            if obj.is_null() {
                None
            } else {
                Some(obj_to_bytes(obj))
            }
        }
    }

    pub fn set_var_bytes(
        &self,
        name1: &[u8],
        name2: Option<&[u8]>,
        value: &[u8],
    ) -> Result<(), TclError> {
        self.set_var_result_bytes(name1, name2, value).map(|_| ())
    }

    pub fn set_var_result_bytes(
        &self,
        name1: &[u8],
        name2: Option<&[u8]>,
        value: &[u8],
    ) -> Result<Vec<u8>, TclError> {
        let c1 = CString::new(name1).map_err(|_| TclError {
            message: "name contains null byte".into(),
            error_info: None,
            error_code: None,
        })?;
        let c2 = match name2 {
            Some(n) => Some(CString::new(n).map_err(|_| TclError {
                message: "name2 contains null byte".into(),
                error_info: None,
                error_code: None,
            })?),
            None => None,
        };
        unsafe {
            let new_obj =
                Tcl_NewStringObj(value.as_ptr() as *const c_char, value.len() as Tcl_Size);
            let result = Tcl_SetVar2Ex(
                self.interp,
                c1.as_ptr(),
                c2.as_ref().map(|c| c.as_ptr()).unwrap_or(std::ptr::null()),
                new_obj,
                TCL_LEAVE_ERR_MSG,
            );
            if result.is_null() {
                Err(self.make_error(get_string_result(self.interp)))
            } else {
                Ok({
                    let mut len: Tcl_Size = 0;
                    let ptr = Tcl_GetStringFromObj(result, &mut len);
                    std::slice::from_raw_parts(ptr as *const u8, len as usize).to_vec()
                })
            }
        }
    }

    fn make_error(&self, message: String) -> TclError {
        // Direct C API failures can set return options before the legacy
        // errorInfo/errorCode variables are synchronized. Read the options
        // themselves, so a variable write never reports a stale error code.
        let (error_info, error_code) = unsafe {
            let options = Tcl_GetReturnOptions(self.interp, TCL_ERROR);
            if options.is_null() {
                (None, None)
            } else {
                let field = |key: &[u8]| {
                    let key_obj =
                        Tcl_NewStringObj(key.as_ptr() as *const c_char, key.len() as Tcl_Size);
                    Tcl_IncrRefCount(key_obj);
                    let mut value = std::ptr::null_mut();
                    let code = Tcl_DictObjGet(std::ptr::null_mut(), options, key_obj, &mut value);
                    Tcl_DecrRefCount(key_obj);
                    (code == TCL_OK && !value.is_null())
                        .then(|| String::from_utf8_lossy(&obj_to_bytes(value)).into_owned())
                };
                let fields = (field(b"-errorinfo"), field(b"-errorcode"));
                Tcl_DecrRefCount(options);
                fields
            }
        };
        TclError {
            message,
            error_info,
            error_code,
        }
    }
}

/// Result of length-aware object-vector evaluation.
#[derive(Debug, Clone)]
pub struct EvalResult {
    pub code: c_int,
    pub result: Vec<u8>,
    pub error_info: Option<Vec<u8>>,
    pub error_code: Option<Vec<u8>>,
    pub return_options: Option<Vec<u8>>,
}

impl Drop for Interpreter {
    fn drop(&mut self) {
        self.remove_trace();
        unsafe {
            Tcl_DeleteInterp(self.interp);
        }
    }
}

// ─── Helper functions ────────────────────────────────────────────────

unsafe fn get_string_result(interp: *mut Tcl_Interp) -> String {
    unsafe {
        let obj = Tcl_GetObjResult(interp);
        if obj.is_null() {
            String::new()
        } else {
            let mut len: Tcl_Size = 0;
            let ptr = Tcl_GetStringFromObj(obj, &mut len);
            if ptr.is_null() {
                String::new()
            } else {
                let slice = std::slice::from_raw_parts(ptr as *const u8, len as usize);
                String::from_utf8_lossy(slice).into_owned()
            }
        }
    }
}

unsafe fn obj_to_string(obj: *mut Tcl_Obj) -> String {
    unsafe { String::from_utf8_lossy(&obj_to_bytes(obj)).into_owned() }
}

unsafe fn obj_to_bytes(obj: *mut Tcl_Obj) -> Vec<u8> {
    unsafe {
        let mut len: Tcl_Size = 0;
        let ptr = Tcl_GetStringFromObj(obj, &mut len);
        if ptr.is_null() || len < 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(ptr as *const u8, len as usize).to_vec()
        }
    }
}

// ─── Trace callback trampoline ───────────────────────────────────────

unsafe extern "C" fn trace_callback_trampoline(
    client_data: *mut c_void,
    _interp: *mut Tcl_Interp,
    level: c_int,
    command: *const c_char,
    _command_token: Tcl_Command,
    _objc: c_int,
    _objv: *const *mut Tcl_Obj,
) -> c_int {
    unsafe {
        let callback = &mut *(client_data as *mut Box<dyn FnMut(&str, i32) -> bool>);
        let cmd_str = CStr::from_ptr(command).to_string_lossy();
        if callback(&cmd_str, level) {
            TCL_OK
        } else {
            TCL_ERROR
        }
    }
}

unsafe extern "C" fn trace_delete_callback(client_data: *mut c_void) {
    unsafe {
        let _ = Box::from_raw(client_data as *mut Box<dyn FnMut(&str, i32) -> bool>);
    }
}
