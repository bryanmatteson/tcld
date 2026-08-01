pub mod nub;
pub mod step_engine;
pub mod watchpoints;
pub mod wrappers;

use std::collections::BTreeMap;
use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_uint, c_void};
use std::sync::{Arc, Mutex};

use tcl_sys::{
    TCL_ERROR, TCL_GLOBAL_ONLY, TCL_OK, Tcl_CmdInfo, Tcl_CreateObjCommand, Tcl_DictObjSize,
    Tcl_EvalEx, Tcl_FindCommand, Tcl_FindNamespace, Tcl_GetCommandFullName,
    Tcl_GetCommandInfoFromToken, Tcl_GetDoubleFromObj, Tcl_GetIntFromObj, Tcl_GetObjResult,
    Tcl_GetStringFromObj, Tcl_GetVar2Ex, Tcl_Interp, Tcl_ListObjLength, Tcl_NewStringObj, Tcl_Obj,
    Tcl_PkgProvide, Tcl_SetObjResult, Tcl_SetVar2Ex,
};

const EXT_VERSION: &str = env!("CARGO_PKG_VERSION");
const ADAPTER_API_VERSION: &str = "1";

unsafe extern "C" {
    fn tcld_opcode_runtime_supported(interp: *mut Tcl_Interp) -> c_int;
    fn tcld_opcode_runtime_set_hook(
        interp: *mut Tcl_Interp,
        hook: Option<
            unsafe extern "C" fn(
                *mut c_void,
                *mut Tcl_Interp,
                tcl_sys::Tcl_Size,
                c_uint,
                tcl_sys::Tcl_Size,
            ) -> c_int,
        >,
        client_data: *mut c_void,
    ) -> c_int;
    fn tcld_opcode_runtime_arm(interp: *mut Tcl_Interp, on: c_int) -> c_int;
    fn tcld_opcode_runtime_current_pc(
        interp: *mut Tcl_Interp,
        pc_out: *mut tcl_sys::Tcl_Size,
    ) -> c_int;
    fn tcld_opcode_runtime_current_source(
        interp: *mut Tcl_Interp,
        file_out: *mut *const c_char,
        line_out: *mut tcl_sys::Tcl_Size,
    ) -> c_int;
}

#[derive(Debug, Clone, Default)]
struct CapabilitySet {
    flags: BTreeMap<&'static str, bool>,
}

impl CapabilitySet {
    fn new() -> Self {
        Self {
            flags: BTreeMap::new(),
        }
    }

    fn with_flag(mut self, key: &'static str, enabled: bool) -> Self {
        self.flags.insert(key, enabled);
        self
    }

    fn insert(&mut self, key: &'static str, enabled: bool) {
        self.flags.insert(key, enabled);
    }

    fn get(&self, key: &str) -> Option<bool> {
        self.flags.get(key).copied()
    }

    fn to_kv_string(&self) -> String {
        let mut parts = Vec::with_capacity(self.flags.len() * 2);
        for (name, enabled) in &self.flags {
            parts.push((*name).to_string());
            parts.push(if *enabled {
                "1".to_string()
            } else {
                "0".to_string()
            });
        }
        parts.join(" ")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeFamily {
    Tcl85,
    Tcl86,
    Tcl90,
    Unknown,
}

impl RuntimeFamily {
    fn as_str(self) -> &'static str {
        match self {
            Self::Tcl85 => "tcl8.5",
            Self::Tcl86 => "tcl8.6",
            Self::Tcl90 => "tcl9.0",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone)]
struct RuntimeInfo {
    family: RuntimeFamily,
    patchlevel: String,
}

trait DebugAdapter {
    fn id(&self) -> &'static str;
    fn capabilities(&self, runtime: &RuntimeInfo) -> CapabilitySet;
    fn install(&self, interp: *mut Tcl_Interp, runtime: &RuntimeInfo) -> Result<(), String>;
    fn step_instruction(
        &self,
        interp: *mut Tcl_Interp,
        runtime: &RuntimeInfo,
    ) -> Result<String, String>;
}

struct Tcl85Adapter;
struct Tcl86Adapter;
struct Tcl90Adapter;
struct UnsupportedAdapter;

impl DebugAdapter for Tcl85Adapter {
    fn id(&self) -> &'static str {
        "adapter_tcl85"
    }

    fn capabilities(&self, _runtime: &RuntimeInfo) -> CapabilitySet {
        CapabilitySet::new()
            .with_flag("command_resolution", false)
            .with_flag("command_step", true)
            .with_flag("instruction_step", false)
            .with_flag("opcode_profiling", false)
            .with_flag("adapter_tcl85", true)
            .with_flag("adapter_tcl86", false)
            .with_flag("adapter_tcl90", false)
            .with_flag("runtime_supported", true)
    }

    fn install(&self, interp: *mut Tcl_Interp, runtime: &RuntimeInfo) -> Result<(), String> {
        if runtime.family != RuntimeFamily::Tcl85 {
            return Err(format!(
                "{} cannot install on {}",
                self.id(),
                runtime.patchlevel
            ));
        }

        let mut capabilities = self.capabilities(runtime);
        capabilities.insert("runtime_tcl8_5", true);
        capabilities.insert("runtime_tcl8_6", false);
        capabilities.insert("runtime_tcl9", false);
        install_common_metadata(interp, self.id(), runtime, &capabilities)?;
        set_global_var(interp, "::tcld::dbg::adapter_variant", "8.5")?;
        Ok(())
    }

    fn step_instruction(
        &self,
        _interp: *mut Tcl_Interp,
        runtime: &RuntimeInfo,
    ) -> Result<String, String> {
        Err(format!(
            "{}: instruction stepping not available for {} ({})",
            self.id(),
            runtime.family.as_str(),
            runtime.patchlevel
        ))
    }
}

impl DebugAdapter for Tcl86Adapter {
    fn id(&self) -> &'static str {
        "adapter_tcl86"
    }

    fn capabilities(&self, _runtime: &RuntimeInfo) -> CapabilitySet {
        CapabilitySet::new()
            .with_flag("command_resolution", false)
            .with_flag("command_step", true)
            .with_flag("instruction_step", false)
            .with_flag("opcode_profiling", false)
            .with_flag("adapter_tcl85", false)
            .with_flag("adapter_tcl86", true)
            .with_flag("adapter_tcl90", false)
            .with_flag("runtime_supported", true)
    }

    fn install(&self, interp: *mut Tcl_Interp, runtime: &RuntimeInfo) -> Result<(), String> {
        if runtime.family != RuntimeFamily::Tcl86 {
            return Err(format!(
                "{} cannot install on {}",
                self.id(),
                runtime.patchlevel
            ));
        }

        let mut capabilities = self.capabilities(runtime);
        capabilities.insert("runtime_tcl8_5", false);
        capabilities.insert("runtime_tcl8_6", true);
        capabilities.insert("runtime_tcl9", false);
        install_common_metadata(interp, self.id(), runtime, &capabilities)?;
        set_global_var(interp, "::tcld::dbg::adapter_variant", "8.6")?;
        Ok(())
    }

    fn step_instruction(
        &self,
        _interp: *mut Tcl_Interp,
        runtime: &RuntimeInfo,
    ) -> Result<String, String> {
        Err(format!(
            "{}: instruction stepping not available for {} ({})",
            self.id(),
            runtime.family.as_str(),
            runtime.patchlevel
        ))
    }
}

impl DebugAdapter for UnsupportedAdapter {
    fn id(&self) -> &'static str {
        "adapter_unknown"
    }

    fn capabilities(&self, _runtime: &RuntimeInfo) -> CapabilitySet {
        CapabilitySet::new()
            .with_flag("command_resolution", false)
            .with_flag("command_step", false)
            .with_flag("instruction_step", false)
            .with_flag("opcode_profiling", false)
            .with_flag("adapter_tcl85", false)
            .with_flag("adapter_tcl86", false)
            .with_flag("adapter_tcl90", false)
            .with_flag("runtime_supported", false)
    }

    fn install(&self, _interp: *mut Tcl_Interp, runtime: &RuntimeInfo) -> Result<(), String> {
        Err(format!("unsupported Tcl runtime: {}", runtime.patchlevel))
    }

    fn step_instruction(
        &self,
        _interp: *mut Tcl_Interp,
        runtime: &RuntimeInfo,
    ) -> Result<String, String> {
        Err(format!("unsupported Tcl runtime: {}", runtime.patchlevel))
    }
}

impl DebugAdapter for Tcl90Adapter {
    fn id(&self) -> &'static str {
        "adapter_tcl90"
    }

    fn capabilities(&self, _runtime: &RuntimeInfo) -> CapabilitySet {
        CapabilitySet::new()
            .with_flag("command_resolution", true)
            .with_flag("command_step", true)
            .with_flag("instruction_step", false)
            .with_flag("opcode_profiling", false)
            .with_flag("obj_introspection", true)
            .with_flag("adapter_tcl85", false)
            .with_flag("adapter_tcl86", false)
            .with_flag("adapter_tcl90", true)
            .with_flag("runtime_supported", true)
    }

    fn install(&self, interp: *mut Tcl_Interp, runtime: &RuntimeInfo) -> Result<(), String> {
        if runtime.family != RuntimeFamily::Tcl90 {
            return Err(format!(
                "{} cannot install on {}",
                self.id(),
                runtime.patchlevel
            ));
        }

        let mut capabilities = self.capabilities(runtime);
        let opcode_supported = opcode_runtime_supported(interp);
        capabilities.insert("instruction_step", opcode_supported);
        capabilities.insert("opcode_profiling", opcode_supported);
        capabilities.insert("runtime_tcl8_5", false);
        capabilities.insert("runtime_tcl8_6", false);
        capabilities.insert("runtime_tcl9", true);
        install_common_metadata(interp, self.id(), runtime, &capabilities)?;
        set_global_var(interp, "::tcld::dbg::adapter_variant", "9.0")?;

        create_command(interp, "::tcld::dbg::inspect_obj", inspect_obj_cmd)?;
        create_command(
            interp,
            "::tcld::dbg::inspect_all_vars",
            inspect_all_vars_cmd,
        )?;
        create_command(interp, "::tcld::dbg::resolve_command", resolve_command_cmd)?;

        install_resolve_helpers(interp)?;
        if opcode_supported {
            opcode_set_hook(interp)?;
            opcode_arm(interp, false)?;
        }

        Ok(())
    }

    fn step_instruction(
        &self,
        interp: *mut Tcl_Interp,
        _runtime: &RuntimeInfo,
    ) -> Result<String, String> {
        if !opcode_runtime_supported(interp) {
            return Err("instruction stepping requires patched Tcl9 opcode runtime".to_string());
        }

        unsafe {
            step_engine::set_opcode_mode(interp, step_engine::OpcodeMode::StepInto)?;
        }
        opcode_arm(interp, true)?;
        unsafe { step_engine::signal_resume(interp) };

        Ok("ok 1".to_string())
    }
}

static TCL85_ADAPTER: Tcl85Adapter = Tcl85Adapter;
static TCL86_ADAPTER: Tcl86Adapter = Tcl86Adapter;
static TCL90_ADAPTER: Tcl90Adapter = Tcl90Adapter;
static UNSUPPORTED_ADAPTER: UnsupportedAdapter = UnsupportedAdapter;

struct AdapterSelection {
    adapter: &'static dyn DebugAdapter,
    runtime: RuntimeInfo,
}

fn make_cstring(text: &str) -> Result<CString, String> {
    CString::new(text).map_err(|_| "string contains NUL byte".to_string())
}

pub(crate) unsafe fn objv_to_string(objv: *const *mut Tcl_Obj, index: usize) -> Option<String> {
    let obj = unsafe { *objv.add(index) };
    if obj.is_null() {
        return None;
    }

    let mut len: tcl_sys::Tcl_Size = 0;
    let ptr = unsafe { Tcl_GetStringFromObj(obj, &mut len) };
    if ptr.is_null() || len < 0 {
        return None;
    }

    let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
    Some(String::from_utf8_lossy(bytes).into_owned())
}

pub(crate) fn set_result(interp: *mut Tcl_Interp, text: &str) -> c_int {
    let Ok(c_text) = make_cstring(text) else {
        return TCL_ERROR;
    };
    unsafe {
        Tcl_SetObjResult(interp, Tcl_NewStringObj(c_text.as_ptr(), -1));
    }
    TCL_OK
}

pub(crate) fn set_error(interp: *mut Tcl_Interp, message: &str) -> c_int {
    let Ok(c_msg) = make_cstring(message) else {
        return TCL_ERROR;
    };
    unsafe {
        Tcl_SetObjResult(interp, Tcl_NewStringObj(c_msg.as_ptr(), -1));
    }
    TCL_ERROR
}

fn opcode_runtime_supported(interp: *mut Tcl_Interp) -> bool {
    unsafe { tcld_opcode_runtime_supported(interp) != 0 }
}

fn opcode_set_hook(interp: *mut Tcl_Interp) -> Result<(), String> {
    let rc = unsafe {
        tcld_opcode_runtime_set_hook(interp, Some(opcode_boundary_hook), std::ptr::null_mut())
    };
    if rc < 0 {
        return Err("opcode runtime hook tcld_opcode_set_hook not available".to_string());
    }
    if rc != TCL_OK {
        return Err(format!(
            "opcode runtime hook registration failed (code {rc})"
        ));
    }
    Ok(())
}

fn opcode_arm(interp: *mut Tcl_Interp, on: bool) -> Result<(), String> {
    let rc = unsafe { tcld_opcode_runtime_arm(interp, if on { 1 } else { 0 }) };
    if rc < 0 {
        return Err("opcode runtime hook tcld_opcode_arm not available".to_string());
    }
    Ok(())
}

fn opcode_current_pc(interp: *mut Tcl_Interp) -> Result<i64, String> {
    let mut pc: tcl_sys::Tcl_Size = -1;
    let rc = unsafe { tcld_opcode_runtime_current_pc(interp, &mut pc as *mut tcl_sys::Tcl_Size) };
    if rc < 0 {
        return Err("opcode runtime hook tcld_current_opcode_pc not available".to_string());
    }
    if rc != TCL_OK {
        return Err(format!("opcode runtime current_pc failed (code {rc})"));
    }
    Ok(pc as i64)
}

fn opcode_current_source(interp: *mut Tcl_Interp) -> Result<Option<(String, i64)>, String> {
    let mut file: *const c_char = std::ptr::null();
    let mut line: tcl_sys::Tcl_Size = 0;
    let rc = unsafe {
        tcld_opcode_runtime_current_source(
            interp,
            &mut file as *mut *const c_char,
            &mut line as *mut tcl_sys::Tcl_Size,
        )
    };
    if rc < 0 {
        return Err("opcode runtime hook tcld_current_source not available".to_string());
    }
    if rc == 0 {
        return Ok(None);
    }
    if file.is_null() {
        return Ok(None);
    }
    let path = unsafe { std::ffi::CStr::from_ptr(file) }
        .to_string_lossy()
        .into_owned();
    Ok(Some((path, line as i64)))
}

unsafe extern "C" fn opcode_boundary_hook(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    pc_offset: tcl_sys::Tcl_Size,
    bc_flags: c_uint,
    num_levels: tcl_sys::Tcl_Size,
) -> c_int {
    unsafe { step_engine::opcode_boundary(interp, pc_offset, bc_flags, num_levels) }
}

fn parse_runtime_family(patchlevel: &str) -> RuntimeFamily {
    let mut parts = patchlevel.split('.');
    let major = parts
        .next()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);
    let minor = parts
        .next()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);

    match (major, minor) {
        (8, 5) => RuntimeFamily::Tcl85,
        (8, 6) => RuntimeFamily::Tcl86,
        (9, _) => RuntimeFamily::Tcl90,
        _ => RuntimeFamily::Unknown,
    }
}

pub(crate) fn runtime_patchlevel(interp: *mut Tcl_Interp) -> String {
    let Ok(var_name) = make_cstring("tcl_patchLevel") else {
        return "unknown".into();
    };

    let obj =
        unsafe { Tcl_GetVar2Ex(interp, var_name.as_ptr(), std::ptr::null(), TCL_GLOBAL_ONLY) };

    if obj.is_null() {
        return "unknown".into();
    }

    let mut len: tcl_sys::Tcl_Size = 0;
    let ptr = unsafe { Tcl_GetStringFromObj(obj, &mut len) };
    if ptr.is_null() || len < 0 {
        return "unknown".into();
    }

    let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
    String::from_utf8_lossy(bytes).into_owned()
}

fn adapter_for_family(family: RuntimeFamily) -> &'static dyn DebugAdapter {
    match family {
        RuntimeFamily::Tcl85 => &TCL85_ADAPTER,
        RuntimeFamily::Tcl86 => &TCL86_ADAPTER,
        RuntimeFamily::Tcl90 => &TCL90_ADAPTER,
        RuntimeFamily::Unknown => &UNSUPPORTED_ADAPTER,
    }
}

fn select_adapter_for_patchlevel(patchlevel: String) -> AdapterSelection {
    let family = parse_runtime_family(&patchlevel);
    AdapterSelection {
        adapter: adapter_for_family(family),
        runtime: RuntimeInfo { family, patchlevel },
    }
}

fn select_adapter(interp: *mut Tcl_Interp) -> AdapterSelection {
    select_adapter_for_patchlevel(runtime_patchlevel(interp))
}

fn resolved_capabilities(interp: *mut Tcl_Interp, selected: &AdapterSelection) -> CapabilitySet {
    let mut caps = selected.adapter.capabilities(&selected.runtime);
    if selected.runtime.family == RuntimeFamily::Tcl90 {
        let enabled = if interp.is_null() {
            false
        } else {
            opcode_runtime_supported(interp)
        };
        caps.insert("instruction_step", enabled);
        caps.insert("opcode_profiling", enabled);
    }
    caps.insert(
        "runtime_tcl8_5",
        selected.runtime.family == RuntimeFamily::Tcl85,
    );
    caps.insert(
        "runtime_tcl8_6",
        selected.runtime.family == RuntimeFamily::Tcl86,
    );
    caps.insert(
        "runtime_tcl9",
        selected.runtime.family == RuntimeFamily::Tcl90,
    );
    caps
}

fn capabilities_dict(interp: *mut Tcl_Interp) -> String {
    let selected = select_adapter(interp);
    let caps = resolved_capabilities(interp, &selected);

    let mut parts = vec![
        format!("adapter {}", selected.adapter.id()),
        format!("adapter_api {}", ADAPTER_API_VERSION),
        format!("runtime_family {}", selected.runtime.family.as_str()),
        format!("runtime_patchlevel {}", selected.runtime.patchlevel),
        format!("extension_version {}", EXT_VERSION),
    ];

    for (name, enabled) in caps.flags {
        parts.push(format!("{name} {}", if enabled { 1 } else { 0 }));
    }

    parts.join(" ")
}

fn interp_error_message(interp: *mut Tcl_Interp) -> String {
    let obj = unsafe { Tcl_GetObjResult(interp) };
    if obj.is_null() {
        return "unknown Tcl error".to_string();
    }
    let mut len: tcl_sys::Tcl_Size = 0;
    let ptr = unsafe { Tcl_GetStringFromObj(obj, &mut len) };
    if ptr.is_null() || len < 0 {
        return "unknown Tcl error".to_string();
    }
    let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
    String::from_utf8_lossy(bytes).into_owned()
}

fn eval_script(interp: *mut Tcl_Interp, script: &str) -> Result<(), String> {
    let c_script = make_cstring(script)?;
    let code = unsafe { Tcl_EvalEx(interp, c_script.as_ptr(), -1, 0) };
    if code == TCL_OK {
        Ok(())
    } else {
        Err(interp_error_message(interp))
    }
}

fn set_global_var(interp: *mut Tcl_Interp, name: &str, value: &str) -> Result<(), String> {
    let c_name = make_cstring(name)?;
    let c_value = make_cstring(value)?;

    let obj = unsafe { Tcl_NewStringObj(c_value.as_ptr(), -1) };
    if obj.is_null() {
        return Err(format!("failed to allocate Tcl object for {name}"));
    }

    let result = unsafe {
        Tcl_SetVar2Ex(
            interp,
            c_name.as_ptr(),
            std::ptr::null(),
            obj,
            TCL_GLOBAL_ONLY,
        )
    };

    if result.is_null() {
        return Err(format!(
            "failed to set {name}: {}",
            interp_error_message(interp)
        ));
    }

    Ok(())
}

fn install_common_metadata(
    interp: *mut Tcl_Interp,
    adapter_id: &str,
    runtime: &RuntimeInfo,
    capabilities: &CapabilitySet,
) -> Result<(), String> {
    eval_script(interp, "namespace eval ::tcld::dbg {}")?;

    set_global_var(interp, "::tcld::dbg::adapter_id", adapter_id)?;
    set_global_var(interp, "::tcld::dbg::adapter_api", ADAPTER_API_VERSION)?;
    set_global_var(
        interp,
        "::tcld::dbg::runtime_family",
        runtime.family.as_str(),
    )?;
    set_global_var(
        interp,
        "::tcld::dbg::runtime_patchlevel",
        &runtime.patchlevel,
    )?;
    set_global_var(interp, "::tcld::dbg::extension_version", EXT_VERSION)?;
    set_global_var(
        interp,
        "::tcld::dbg::capabilities",
        &capabilities.to_kv_string(),
    )?;
    set_global_var(interp, "::tcld::dbg::installed", "1")?;

    Ok(())
}

unsafe extern "C" fn capabilities_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    _objv: *const *mut Tcl_Obj,
) -> c_int {
    if objc != 1 {
        return set_error(interp, "usage: tcld::dbg::capabilities");
    }
    set_result(interp, &capabilities_dict(interp))
}

unsafe extern "C" fn install_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    _objv: *const *mut Tcl_Obj,
) -> c_int {
    if objc != 1 {
        return set_error(interp, "usage: tcld::dbg::install");
    }

    let selected = select_adapter(interp);
    match selected.adapter.install(interp, &selected.runtime) {
        Ok(()) => set_result(
            interp,
            &format!(
                "installed 1 adapter {} runtime {} api {}",
                selected.adapter.id(),
                selected.runtime.patchlevel,
                ADAPTER_API_VERSION,
            ),
        ),
        Err(err) => set_error(interp, &err),
    }
}

unsafe extern "C" fn step_instruction_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    _objv: *const *mut Tcl_Obj,
) -> c_int {
    if objc != 1 {
        return set_error(interp, "usage: tcld::dbg::step_instruction");
    }
    if let Err(err) = require_debugger_stopped(interp) {
        return set_error(interp, &err);
    }

    let selected = select_adapter(interp);
    match selected.adapter.step_instruction(interp, &selected.runtime) {
        Ok(result) => set_result(interp, &result),
        Err(err) => set_error(interp, &err),
    }
}

unsafe extern "C" fn step_over_instruction_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    _objv: *const *mut Tcl_Obj,
) -> c_int {
    if objc != 1 {
        return set_error(interp, "usage: tcld::dbg::step_over_instruction");
    }
    start_opcode_resume(interp, step_engine::OpcodeResumeRequest::StepOver)
}

unsafe extern "C" fn step_out_instruction_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    _objv: *const *mut Tcl_Obj,
) -> c_int {
    if objc != 1 {
        return set_error(interp, "usage: tcld::dbg::step_out_instruction");
    }
    start_opcode_resume(interp, step_engine::OpcodeResumeRequest::StepOut)
}

unsafe extern "C" fn run_to_pc_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    if objc != 2 {
        return set_error(interp, "usage: tcld::dbg::run_to_pc pc");
    }
    let Some(pc) = (unsafe { objv_to_int(objv, 1) }) else {
        return set_error(interp, "pc must be an integer");
    };
    if pc < 0 {
        return set_error(interp, "pc must be non-negative");
    }
    start_opcode_resume(
        interp,
        step_engine::OpcodeResumeRequest::RunToPc { pc: pc as u64 },
    )
}

unsafe extern "C" fn profile_start_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    if objc != 2 {
        return set_error(interp, "usage: tcld::dbg::profile_start mode");
    }
    let mode = unsafe { objv_to_string(objv, 1) }.unwrap_or_default();
    if mode != "opcode_counts" {
        return set_error(
            interp,
            "unsupported profile mode for opcode runtime: expected opcode_counts",
        );
    }
    if !opcode_runtime_supported(interp) {
        return set_error(
            interp,
            "opcode_counts profiling requires patched Tcl9 opcode runtime",
        );
    }
    if let Err(err) = unsafe { step_engine::profile_start_opcode_counts(interp) } {
        return set_error(interp, &err);
    }
    if let Err(err) = resync_opcode_arm(interp) {
        return set_error(interp, &err);
    }
    set_result(interp, "ok")
}

unsafe extern "C" fn profile_stop_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    _objv: *const *mut Tcl_Obj,
) -> c_int {
    if objc != 1 {
        return set_error(interp, "usage: tcld::dbg::profile_stop");
    }
    if let Err(err) = unsafe { step_engine::profile_stop(interp) } {
        return set_error(interp, &err);
    }
    if let Err(err) = resync_opcode_arm(interp) {
        return set_error(interp, &err);
    }
    set_result(interp, "ok")
}

unsafe extern "C" fn profile_reset_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    _objv: *const *mut Tcl_Obj,
) -> c_int {
    if objc != 1 {
        return set_error(interp, "usage: tcld::dbg::profile_reset");
    }
    if let Err(err) = unsafe { step_engine::profile_reset(interp) } {
        return set_error(interp, &err);
    }
    set_result(interp, "ok")
}

unsafe extern "C" fn profile_data_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    _objv: *const *mut Tcl_Obj,
) -> c_int {
    if objc != 1 {
        return set_error(interp, "usage: tcld::dbg::profile_data");
    }
    let snapshot = match step_engine::opcode_profile_snapshot(interp) {
        Ok(snapshot) => snapshot,
        Err(err) => return set_error(interp, &err),
    };
    let data = snapshot
        .into_iter()
        .map(|(pc, count)| format!("{pc} {count}"))
        .collect::<Vec<_>>()
        .join(" ");
    set_result(interp, &format!("mode opcode_counts data {{{data}}}"))
}

fn require_debugger_stopped(interp: *mut Tcl_Interp) -> Result<(), String> {
    let state = step_engine_state(interp).ok_or_else(|| "step engine not installed".to_string())?;
    let guard = state
        .lock()
        .map_err(|_| "state mutex poisoned".to_string())?;
    guard.require_stopped().map_err(|err| err.to_string())
}

fn start_opcode_resume(
    interp: *mut Tcl_Interp,
    request: step_engine::OpcodeResumeRequest,
) -> c_int {
    if !opcode_runtime_supported(interp) {
        return set_error(
            interp,
            "instruction stepping requires patched Tcl9 opcode runtime",
        );
    }
    if let Err(err) = require_debugger_stopped(interp) {
        return set_error(interp, &err);
    }
    let mode = match step_engine::opcode_mode_for_resume_request(
        request,
        step_engine::paused_opcode_level(interp),
    ) {
        Ok(mode) => mode,
        Err(err) => return set_error(interp, &err),
    };
    if let Err(err) = unsafe { step_engine::set_opcode_mode(interp, mode) } {
        return set_error(interp, &err);
    }
    if let Err(err) = opcode_arm(interp, true) {
        return set_error(interp, &err);
    }
    unsafe { step_engine::signal_resume(interp) };
    set_result(interp, "ok")
}

unsafe extern "C" fn current_pc_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    _objv: *const *mut Tcl_Obj,
) -> c_int {
    if objc != 1 {
        return set_error(interp, "usage: tcld::dbg::current_pc");
    }
    if !opcode_runtime_supported(interp) {
        return set_error(
            interp,
            "current_pc unavailable: patched Tcl9 opcode runtime not detected",
        );
    }
    match opcode_current_pc(interp) {
        Ok(pc) => set_result(interp, &pc.to_string()),
        Err(err) => set_error(interp, &err),
    }
}

unsafe extern "C" fn version_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    _objv: *const *mut Tcl_Obj,
) -> c_int {
    if objc != 1 {
        return set_error(interp, "usage: tcld::dbg::version");
    }
    set_result(interp, EXT_VERSION)
}

unsafe extern "C" fn has_capability_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    if objc != 2 {
        return set_error(interp, "usage: tcld::dbg::has <capability>");
    }

    let capability = unsafe { objv_to_string(objv, 1) }.unwrap_or_default();
    let selected = select_adapter(interp);
    let caps = resolved_capabilities(interp, &selected);
    let enabled = caps.get(capability.as_str()).unwrap_or(false);

    set_result(interp, if enabled { "1" } else { "0" })
}

// ─── Object introspection ────────────────────────────────────────────

struct ObjIntrep {
    type_name: String,
    has_string_rep: bool,
    has_internal_rep: bool,
    ref_count: u32,
    detail: String,
}

fn inspect_obj_ptr(interp: *mut Tcl_Interp, obj: *mut Tcl_Obj) -> ObjIntrep {
    let repr_text = unsafe {
        let repr_var = make_cstring("__tcld_repr_obj").unwrap();
        Tcl_SetVar2Ex(
            interp,
            repr_var.as_ptr(),
            std::ptr::null(),
            obj,
            TCL_GLOBAL_ONLY,
        );

        let script = "::tcl::unsupported::representation $__tcld_repr_obj";
        let c_script = make_cstring(script).unwrap();
        let ret = Tcl_EvalEx(interp, c_script.as_ptr(), -1, 0);
        if ret != TCL_OK {
            return ObjIntrep {
                type_name: "unknown".into(),
                has_string_rep: tcl_sys::has_string_rep(obj),
                has_internal_rep: false,
                ref_count: 0,
                detail: String::new(),
            };
        }
        let result_obj = Tcl_GetObjResult(interp);
        if result_obj.is_null() {
            return ObjIntrep {
                type_name: "unknown".into(),
                has_string_rep: true,
                has_internal_rep: false,
                ref_count: 0,
                detail: String::new(),
            };
        }
        let mut len: tcl_sys::Tcl_Size = 0;
        let ptr = Tcl_GetStringFromObj(result_obj, &mut len);
        if ptr.is_null() || len < 0 {
            return ObjIntrep {
                type_name: "unknown".into(),
                has_string_rep: true,
                has_internal_rep: false,
                ref_count: 0,
                detail: String::new(),
            };
        }
        let bytes = std::slice::from_raw_parts(ptr as *const u8, len as usize);
        String::from_utf8_lossy(bytes).into_owned()
    };

    let has_string_rep = unsafe { tcl_sys::has_string_rep(obj) };
    let mut info = parse_representation(&repr_text, has_string_rep);

    info.detail = build_type_detail(interp, obj, &info.type_name);
    info
}

fn parse_representation(repr: &str, has_string_rep: bool) -> ObjIntrep {
    let mut info = ObjIntrep {
        type_name: "none".into(),
        has_string_rep,
        has_internal_rep: false,
        ref_count: 1,
        detail: String::new(),
    };

    if repr.contains("pure string") {
        info.type_name = "none".into();
        info.has_internal_rep = false;
    } else if let Some(start) = repr.find("value is a ") {
        let after = &repr[start + 11..];
        if let Some(end) = after.find(" with a refcount") {
            info.type_name = after[..end].to_string();
            info.has_internal_rep = true;
        }
    }

    if let Some(start) = repr.find("refcount of ") {
        let after = &repr[start + 12..];
        if let Some(end) = after.find(',')
            && let Ok(rc) = after[..end].trim().parse::<u32>()
        {
            info.ref_count = rc;
        }
    }

    info
}

fn build_type_detail(interp: *mut Tcl_Interp, obj: *mut Tcl_Obj, type_name: &str) -> String {
    match type_name {
        "list" | "arithSeries" => {
            let mut len: tcl_sys::Tcl_Size = 0;
            let ret = unsafe { Tcl_ListObjLength(interp, obj, &mut len) };
            if ret == TCL_OK {
                format!("elements={len}")
            } else {
                String::new()
            }
        }
        "dict" => {
            let mut size: tcl_sys::Tcl_Size = 0;
            let ret = unsafe { Tcl_DictObjSize(interp, obj, &mut size) };
            if ret == TCL_OK {
                format!("entries={size}")
            } else {
                String::new()
            }
        }
        "int" | "wideInt" => {
            let mut val: c_int = 0;
            let ret = unsafe { Tcl_GetIntFromObj(std::ptr::null_mut(), obj, &mut val) };
            if ret == TCL_OK {
                format!("value={val}")
            } else {
                String::new()
            }
        }
        "double" => {
            let mut val: f64 = 0.0;
            let ret = unsafe { Tcl_GetDoubleFromObj(std::ptr::null_mut(), obj, &mut val) };
            if ret == TCL_OK {
                format!("value={val}")
            } else {
                String::new()
            }
        }
        "boolean" => {
            let mut val: c_int = 0;
            let ret =
                unsafe { tcl_sys::Tcl_GetBooleanFromObj(std::ptr::null_mut(), obj, &mut val) };
            if ret == TCL_OK {
                format!("value={}", if val != 0 { "true" } else { "false" })
            } else {
                String::new()
            }
        }
        _ => String::new(),
    }
}

pub(crate) fn tcl_escape_braces(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('{', "\\{")
        .replace('}', "\\}")
}

fn format_intrep_dict(info: &ObjIntrep) -> String {
    format!(
        "type {{{}}} hasStringRep {} hasInternalRep {} refCount {} detail {{{}}}",
        tcl_escape_braces(&info.type_name),
        if info.has_string_rep { 1 } else { 0 },
        if info.has_internal_rep { 1 } else { 0 },
        info.ref_count,
        tcl_escape_braces(&info.detail),
    )
}

unsafe fn resolve_var_obj(interp: *mut Tcl_Interp, var_name: &str, level: u32) -> *mut Tcl_Obj {
    let script = format!(
        "uplevel #{level} [list set {{{}}}]",
        var_name.replace('{', "\\{").replace('}', "\\}")
    );
    let Ok(c_script) = make_cstring(&script) else {
        return std::ptr::null_mut();
    };
    let ret = unsafe { Tcl_EvalEx(interp, c_script.as_ptr(), -1, 0) };
    if ret != TCL_OK {
        return std::ptr::null_mut();
    }
    unsafe { Tcl_GetObjResult(interp) }
}

unsafe extern "C" fn inspect_obj_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    if !(2..=3).contains(&objc) {
        return set_error(interp, "usage: tcld::dbg::inspect_obj varName ?level?");
    }

    let var_name = unsafe { objv_to_string(objv, 1) }.unwrap_or_default();
    let level = if objc == 3 {
        unsafe { objv_to_string(objv, 2) }
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0)
    } else {
        0
    };

    let obj = unsafe { resolve_var_obj(interp, &var_name, level) };
    if obj.is_null() {
        return set_error(interp, &format!("cannot inspect variable: {var_name}"));
    }

    let info = inspect_obj_ptr(interp, obj);
    set_result(interp, &format_intrep_dict(&info))
}

unsafe extern "C" fn inspect_all_vars_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    if objc > 2 {
        return set_error(interp, "usage: tcld::dbg::inspect_all_vars ?level?");
    }

    let level = if objc == 2 {
        unsafe { objv_to_string(objv, 1) }
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0)
    } else {
        0
    };

    let vars_script = format!("uplevel #{level} {{info vars}}");
    let Ok(c_script) = make_cstring(&vars_script) else {
        return set_error(interp, "failed to build vars script");
    };
    let ret = unsafe { Tcl_EvalEx(interp, c_script.as_ptr(), -1, 0) };
    if ret != TCL_OK {
        return set_error(interp, "failed to list variables");
    }

    let vars_result = {
        let result_obj = unsafe { Tcl_GetObjResult(interp) };
        if result_obj.is_null() {
            return set_result(interp, "");
        }
        let mut len: tcl_sys::Tcl_Size = 0;
        let ptr = unsafe { Tcl_GetStringFromObj(result_obj, &mut len) };
        if ptr.is_null() || len < 0 {
            return set_result(interp, "");
        }
        let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
        String::from_utf8_lossy(bytes).into_owned()
    };

    let var_names = split_tcl_list_simple(&vars_result);
    let mut entries = Vec::with_capacity(var_names.len());

    for var_name in &var_names {
        if var_name.starts_with("__tcld_") {
            continue;
        }

        let is_array_script = format!(
            "uplevel #{level} [list array exists {{{}}}]",
            var_name.replace('{', "\\{").replace('}', "\\}")
        );
        let is_array = eval_to_bool(interp, &is_array_script);
        if is_array {
            entries.push(format!(
                "{{name {{{}}} type array hasStringRep 0 hasInternalRep 0 refCount 0 detail {{}}}}",
                tcl_escape_braces(var_name)
            ));
            continue;
        }

        let obj = unsafe { resolve_var_obj(interp, var_name, level) };
        if obj.is_null() {
            continue;
        }

        let info = inspect_obj_ptr(interp, obj);
        entries.push(format!(
            "{{name {{{}}} {}}}",
            tcl_escape_braces(var_name),
            format_intrep_dict(&info)
        ));
    }

    set_result(interp, &entries.join(" "))
}

// ─── Command resolution ──────────────────────────────────────────────

fn install_resolve_helpers(interp: *mut Tcl_Interp) -> Result<(), String> {
    let script = r#"
proc ::tcld::dbg::_resolve_import_chain {name ns} {
    set chain [list]
    set current $name
    set seen [dict create]
    set limit 20
    while {$limit > 0} {
        incr limit -1
        if {[dict exists $seen $current]} break
        dict set seen $current 1
        lappend chain $current
        if {[catch {set origin [namespace eval $ns [list namespace origin $current]]}]} break
        if {$origin eq $current} break
        set current $origin
    }
    return $chain
}
"#;
    let c_script = make_cstring(script)?;
    let ret = unsafe { Tcl_EvalEx(interp, c_script.as_ptr(), -1, 0) };
    if ret != TCL_OK {
        return Err(format!(
            "failed to install resolve helpers: {}",
            interp_error_message(interp)
        ));
    }
    Ok(())
}

unsafe fn get_command_full_name(interp: *mut Tcl_Interp, cmd: tcl_sys::Tcl_Command) -> String {
    unsafe {
        let name_obj = Tcl_NewStringObj(std::ptr::null(), 0);
        Tcl_GetCommandFullName(interp, cmd, name_obj);
        let mut len: tcl_sys::Tcl_Size = 0;
        let ptr = Tcl_GetStringFromObj(name_obj, &mut len);
        if ptr.is_null() || len <= 0 {
            return String::new();
        }
        let bytes = std::slice::from_raw_parts(ptr as *const u8, len as usize);
        String::from_utf8_lossy(bytes).into_owned()
    }
}

unsafe fn get_command_type(cmd: tcl_sys::Tcl_Command) -> &'static str {
    unsafe {
        let mut info: Tcl_CmdInfo = std::mem::zeroed();
        if Tcl_GetCommandInfoFromToken(cmd, &mut info) == 0 {
            return "unknown";
        }
        if info.is_native_object_proc != 0 {
            "native"
        } else if info.obj_proc.is_some() || info.proc_.is_some() {
            "proc"
        } else {
            "unknown"
        }
    }
}

fn eval_to_string(interp: *mut Tcl_Interp, script: &str) -> Option<String> {
    let Ok(c_script) = make_cstring(script) else {
        return None;
    };
    let ret = unsafe { Tcl_EvalEx(interp, c_script.as_ptr(), -1, 0) };
    if ret != TCL_OK {
        return None;
    }
    let result_obj = unsafe { Tcl_GetObjResult(interp) };
    if result_obj.is_null() {
        return None;
    }
    let mut len: tcl_sys::Tcl_Size = 0;
    let ptr = unsafe { Tcl_GetStringFromObj(result_obj, &mut len) };
    if ptr.is_null() || len < 0 {
        return None;
    }
    let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
    Some(String::from_utf8_lossy(bytes).into_owned())
}

fn effective_command_type(
    base_type: &str,
    is_imported: bool,
    is_ensemble: bool,
    is_alias: bool,
) -> String {
    if is_alias {
        "alias".to_string()
    } else if is_ensemble {
        "ensemble".to_string()
    } else if is_imported {
        "import".to_string()
    } else {
        base_type.to_string()
    }
}

fn lookup_alias_target(interp: *mut Tcl_Interp, command_name: &str) -> Option<String> {
    let script = format!("interp alias {{}} {{{}}}", tcl_escape_braces(command_name));
    eval_to_string(interp, &script).filter(|s| !s.is_empty())
}

unsafe extern "C" fn resolve_command_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    if !(2..=3).contains(&objc) {
        return set_error(interp, "usage: tcld::dbg::resolve_command name ?namespace?");
    }

    let name = unsafe { objv_to_string(objv, 1) }.unwrap_or_default();
    let ns_arg = if objc == 3 {
        unsafe { objv_to_string(objv, 2) }.unwrap_or_default()
    } else {
        String::new()
    };

    let resolve_ns = if ns_arg.is_empty() {
        eval_to_string(interp, "namespace current").unwrap_or_else(|| "::".into())
    } else {
        ns_arg.clone()
    };

    let mut resolution_path = Vec::new();

    let ns_ptr = if !ns_arg.is_empty() {
        let Ok(c_ns) = make_cstring(&ns_arg) else {
            return set_error(interp, "invalid namespace name");
        };
        let ns = unsafe { Tcl_FindNamespace(interp, c_ns.as_ptr(), std::ptr::null_mut(), 0) };
        if ns.is_null() {
            return set_result(
                interp,
                &format_resolution_dict(
                    &name,
                    "",
                    &resolve_ns,
                    "",
                    false,
                    None,
                    &[],
                    false,
                    None,
                    false,
                    None,
                    "unknown",
                    false,
                    &resolution_path,
                ),
            );
        }
        ns
    } else {
        std::ptr::null_mut()
    };

    let Ok(c_name) = make_cstring(&name) else {
        return set_error(interp, "invalid command name");
    };
    let cmd = unsafe { Tcl_FindCommand(interp, c_name.as_ptr(), ns_ptr, 0) };

    if cmd.is_null() {
        resolution_path.push(format!(
            "{{step lookup namespace {{{}}} name {{{}}} result not_found}}",
            tcl_escape_braces(&resolve_ns),
            tcl_escape_braces(&name),
        ));
        return set_result(
            interp,
            &format_resolution_dict(
                &name,
                "",
                &resolve_ns,
                "",
                false,
                None,
                &[],
                false,
                None,
                false,
                None,
                "unknown",
                false,
                &resolution_path,
            ),
        );
    }

    let full_name = unsafe { get_command_full_name(interp, cmd) };
    let base_command_type = unsafe { get_command_type(cmd) };

    resolution_path.push(format!(
        "{{step lookup namespace {{{}}} name {{{}}} result found}}",
        tcl_escape_braces(&resolve_ns),
        tcl_escape_braces(&name),
    ));

    let _resolved_ns = extract_namespace(&full_name);

    let import_chain_script = format!(
        "::tcld::dbg::_resolve_import_chain {{{}}} {{{}}}",
        tcl_escape_braces(&full_name),
        tcl_escape_braces(&resolve_ns),
    );
    let import_chain: Vec<String> = eval_to_string(interp, &import_chain_script)
        .map(|s| split_tcl_list_simple(&s))
        .unwrap_or_default();

    let is_imported = import_chain.len() > 1;
    let import_source = if is_imported {
        import_chain.get(1).cloned()
    } else {
        None
    };

    let terminal_name = import_chain
        .last()
        .cloned()
        .unwrap_or_else(|| full_name.clone());

    if is_imported {
        for (i, step_name) in import_chain.iter().enumerate() {
            let result_str = if i == import_chain.len() - 1 {
                "terminal"
            } else {
                "found"
            };
            resolution_path.push(format!(
                "{{step import_resolution source {{{}}} result {result_str}}}",
                tcl_escape_braces(step_name),
            ));
        }
    }

    let is_ensemble_script = format!(
        "namespace ensemble exists {{{}}}",
        tcl_escape_braces(&terminal_name),
    );
    let is_ensemble = eval_to_bool(interp, &is_ensemble_script);

    let ensemble_ns = if is_ensemble {
        let ens_ns_script = format!(
            "namespace ensemble configure {{{}}} -namespace",
            tcl_escape_braces(&terminal_name),
        );
        eval_to_string(interp, &ens_ns_script)
    } else {
        None
    };

    if is_ensemble {
        resolution_path.push(format!(
            "{{step ensemble namespace {{{}}} result found}}",
            tcl_escape_braces(ensemble_ns.as_deref().unwrap_or("")),
        ));
    }

    let alias_target = lookup_alias_target(interp, &full_name)
        .or_else(|| {
            if terminal_name != full_name {
                lookup_alias_target(interp, &terminal_name)
            } else {
                None
            }
        })
        .or_else(|| {
            if name != full_name && name != terminal_name {
                lookup_alias_target(interp, &name)
            } else {
                None
            }
        });
    let is_alias = alias_target.is_some();

    if is_alias {
        resolution_path.push(format!(
            "{{step alias target {{{}}} result found}}",
            tcl_escape_braces(alias_target.as_deref().unwrap_or("")),
        ));
    }

    let final_resolved = if is_imported {
        &terminal_name
    } else {
        &full_name
    };
    let final_ns = extract_namespace(final_resolved);
    let command_type =
        effective_command_type(base_command_type, is_imported, is_ensemble, is_alias);

    set_result(
        interp,
        &format_resolution_dict(
            &name,
            final_resolved,
            &resolve_ns,
            &final_ns,
            is_imported,
            import_source.as_deref(),
            &import_chain,
            is_ensemble,
            ensemble_ns.as_deref(),
            is_alias,
            alias_target.as_deref(),
            &command_type,
            true,
            &resolution_path,
        ),
    )
}

fn extract_namespace(fqn: &str) -> String {
    if let Some(pos) = fqn.rfind("::") {
        let ns = &fqn[..pos];
        if ns.is_empty() {
            "::".to_string()
        } else {
            ns.to_string()
        }
    } else {
        "::".to_string()
    }
}

#[allow(clippy::too_many_arguments)]
fn format_resolution_dict(
    original_name: &str,
    resolved_name: &str,
    namespace: &str,
    resolved_in_namespace: &str,
    is_imported: bool,
    import_source: Option<&str>,
    import_chain: &[String],
    is_ensemble: bool,
    ensemble_namespace: Option<&str>,
    is_alias: bool,
    alias_target: Option<&str>,
    command_type: &str,
    exists: bool,
    resolution_path: &[String],
) -> String {
    let mut parts = Vec::new();

    parts.push(format!(
        "originalName {{{}}}",
        tcl_escape_braces(original_name)
    ));
    parts.push(format!(
        "resolvedName {{{}}}",
        tcl_escape_braces(resolved_name)
    ));
    parts.push(format!(
        "fullyQualifiedName {{{}}}",
        tcl_escape_braces(resolved_name)
    ));
    parts.push(format!("namespace {{{}}}", tcl_escape_braces(namespace)));
    parts.push(format!(
        "resolvedInNamespace {{{}}}",
        tcl_escape_braces(resolved_in_namespace)
    ));
    parts.push(format!("isImported {}", if is_imported { 1 } else { 0 }));

    if let Some(src) = import_source {
        parts.push(format!("importSource {{{}}}", tcl_escape_braces(src)));
    }

    let chain_str = import_chain
        .iter()
        .map(|s| format!("{{{}}}", tcl_escape_braces(s)))
        .collect::<Vec<_>>()
        .join(" ");
    parts.push(format!("importChain {{{chain_str}}}"));

    parts.push(format!("isEnsemble {}", if is_ensemble { 1 } else { 0 }));
    if let Some(ens_ns) = ensemble_namespace {
        parts.push(format!(
            "ensembleNamespace {{{}}}",
            tcl_escape_braces(ens_ns)
        ));
    }

    parts.push(format!("isAlias {}", if is_alias { 1 } else { 0 }));
    if let Some(target) = alias_target {
        parts.push(format!("aliasTarget {{{}}}", tcl_escape_braces(target)));
    }

    parts.push(format!(
        "commandType {{{}}}",
        tcl_escape_braces(command_type)
    ));
    parts.push(format!("exists {}", if exists { 1 } else { 0 }));

    let path_str = resolution_path.join(" ");
    parts.push(format!("resolutionPath {{{path_str}}}"));

    parts.join(" ")
}

fn split_tcl_list_simple(s: &str) -> Vec<String> {
    let s = s.trim();
    if s.is_empty() {
        return vec![];
    }
    let mut items = vec![];
    let mut chars = s.chars().peekable();
    while chars.peek().is_some() {
        while chars.peek() == Some(&' ') || chars.peek() == Some(&'\t') {
            chars.next();
        }
        if chars.peek().is_none() {
            break;
        }
        if chars.peek() == Some(&'{') {
            chars.next();
            let mut depth = 1;
            let mut item = String::new();
            for c in chars.by_ref() {
                if c == '{' {
                    depth += 1;
                    item.push(c);
                } else if c == '}' {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    item.push(c);
                } else {
                    item.push(c);
                }
            }
            items.push(item);
        } else {
            let mut item = String::new();
            while let Some(&c) = chars.peek() {
                if c == ' ' || c == '\t' {
                    break;
                }
                item.push(c);
                chars.next();
            }
            items.push(item);
        }
    }
    items
}

fn eval_to_bool(interp: *mut Tcl_Interp, script: &str) -> bool {
    let Ok(c_script) = make_cstring(script) else {
        return false;
    };
    let ret = unsafe { Tcl_EvalEx(interp, c_script.as_ptr(), -1, 0) };
    if ret != TCL_OK {
        return false;
    }
    let result_obj = unsafe { Tcl_GetObjResult(interp) };
    if result_obj.is_null() {
        return false;
    }
    let mut len: tcl_sys::Tcl_Size = 0;
    let ptr = unsafe { Tcl_GetStringFromObj(result_obj, &mut len) };
    if ptr.is_null() || len < 0 {
        return false;
    }
    let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
    let text = String::from_utf8_lossy(bytes);
    text.trim() == "1"
}

fn create_command(
    interp: *mut Tcl_Interp,
    name: &str,
    proc_fn: unsafe extern "C" fn(
        *mut c_void,
        *mut Tcl_Interp,
        c_int,
        *const *mut Tcl_Obj,
    ) -> c_int,
) -> Result<(), String> {
    let cname = make_cstring(name)?;
    unsafe {
        Tcl_CreateObjCommand(
            interp,
            cname.as_ptr(),
            Some(proc_fn),
            std::ptr::null_mut(),
            None,
        );
    }
    Ok(())
}

// ─── Step engine FFI bridge ──────────────────────────────────────────
//
// One DebugState per Tcl_Interp. The host (test harness, embedding app)
// installs a step engine on a freshly-created interp via
// ::tcld::dbg::engine_install which binds a fresh DebugState. All other
// step / breakpoint commands route through that binding.

use tcld_debug_state::{BlockId, BreakpointKind, BreakpointStore, DebugState, Location};

pub(crate) fn step_engine_state(interp: *mut Tcl_Interp) -> Option<Arc<Mutex<DebugState>>> {
    let ctx_ptr = step_engine::lookup_ctx(interp)?;
    Some(unsafe { (*ctx_ptr).state.clone() })
}

unsafe extern "C" fn engine_install_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    _objc: c_int,
    _objv: *const *mut Tcl_Obj,
) -> c_int {
    let state = Arc::new(Mutex::new(DebugState::new()));
    if let Err(err) = unsafe { step_engine::install(interp, state) } {
        return set_error(interp, &err);
    }
    if let Err(err) = unsafe { install_tier2_helpers(interp) } {
        return set_error(interp, &err);
    }
    set_result(interp, "ok")
}

unsafe extern "C" fn engine_uninstall_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    _objc: c_int,
    _objv: *const *mut Tcl_Obj,
) -> c_int {
    match unsafe { step_engine::uninstall(interp) } {
        Ok(()) => set_result(interp, "ok"),
        Err(err) => set_error(interp, &err),
    }
}

unsafe extern "C" fn step_into_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    _objc: c_int,
    _objv: *const *mut Tcl_Obj,
) -> c_int {
    let Some(state) = step_engine_state(interp) else {
        return set_error(interp, "step engine not installed");
    };
    {
        let guard = match state.lock() {
            Ok(g) => g,
            Err(_) => return set_error(interp, "state mutex poisoned"),
        };
        if let Err(err) = guard.require_stopped() {
            return set_error(interp, &err.to_string());
        }
    }
    if let Err(err) =
        unsafe { step_engine::set_step_mode(interp, step_engine::StepMode::StepInto, 0) }
    {
        return set_error(interp, &err);
    }
    if let Err(err) = sync_opcode_arm_for_non_opcode_resume(interp) {
        return set_error(interp, &err);
    }
    unsafe { step_engine::signal_resume(interp) };
    set_result(interp, "ok")
}

unsafe extern "C" fn step_over_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    _objc: c_int,
    _objv: *const *mut Tcl_Obj,
) -> c_int {
    set_step_with_anchor(interp, step_engine::StepMode::StepOver)
}

unsafe extern "C" fn step_out_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    _objc: c_int,
    _objv: *const *mut Tcl_Obj,
) -> c_int {
    set_step_with_anchor(interp, step_engine::StepMode::StepOut)
}

unsafe extern "C" fn continue_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    _objc: c_int,
    _objv: *const *mut Tcl_Obj,
) -> c_int {
    let Some(state) = step_engine_state(interp) else {
        return set_error(interp, "step engine not installed");
    };
    {
        let guard = match state.lock() {
            Ok(g) => g,
            Err(_) => return set_error(interp, "state mutex poisoned"),
        };
        if let Err(err) = guard.require_stopped() {
            return set_error(interp, &err.to_string());
        }
    }
    if let Err(err) =
        unsafe { step_engine::set_step_mode(interp, step_engine::StepMode::Continue, 0) }
    {
        return set_error(interp, &err);
    }
    if let Err(err) = sync_opcode_arm_for_non_opcode_resume(interp) {
        return set_error(interp, &err);
    }
    unsafe { step_engine::signal_resume(interp) };
    set_result(interp, "ok")
}

fn set_step_with_anchor(interp: *mut Tcl_Interp, mode: step_engine::StepMode) -> c_int {
    let Some(state) = step_engine_state(interp) else {
        return set_error(interp, "step engine not installed");
    };
    {
        let guard = match state.lock() {
            Ok(g) => g,
            Err(_) => return set_error(interp, "state mutex poisoned"),
        };
        if let Err(err) = guard.require_stopped() {
            return set_error(interp, &err.to_string());
        }
    }
    // Anchor to the trace level of the PAUSED command, recorded by the
    // trace handler when it paused. Sampling `info level` here would be
    // wrong twice over: proc-frame depth is a different scale than the
    // `Tcl_CreateObjTrace` command-nesting level the handler compares
    // the anchor against, and any "current" depth read at step-command
    // time measures the dispatch context (the nub's synchronous pause
    // loop, a vwait event callback, a subscriber) instead of the paused
    // user frame — so StepOver/StepOut would never match again.
    let Some(level) = step_engine::paused_trace_level(interp) else {
        // require_stopped passed but no pause is in progress (e.g. a
        // host forced app_state without a real trace pause). There is
        // no paused frame to step relative to.
        return set_error(
            interp,
            "cannot step: not paused at a traced command (no paused frame recorded)",
        );
    };
    if let Err(err) = unsafe { step_engine::set_step_mode(interp, mode, level) } {
        return set_error(interp, &err);
    }
    if let Err(err) = sync_opcode_arm_for_non_opcode_resume(interp) {
        return set_error(interp, &err);
    }
    unsafe { step_engine::signal_resume(interp) };
    set_result(interp, "ok")
}

fn sync_opcode_arm_for_non_opcode_resume(interp: *mut Tcl_Interp) -> Result<(), String> {
    if opcode_runtime_supported(interp) {
        unsafe {
            step_engine::set_opcode_mode(interp, step_engine::OpcodeMode::Continue)?;
        }
        resync_opcode_arm(interp)?;
    }
    Ok(())
}

fn resync_opcode_arm(interp: *mut Tcl_Interp) -> Result<(), String> {
    if opcode_runtime_supported(interp) {
        opcode_arm(interp, step_engine::opcode_hook_needed(interp))?;
    }
    Ok(())
}

unsafe extern "C" fn breakpoint_add_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    // Usage:
    //   ::tcld::dbg::breakpoint_add <proc_qname> <body_byte_offset> <body_byte_len> ?<condition>?
    //   ::tcld::dbg::breakpoint_add source <proc_qname> <body_byte_offset> <body_byte_len> ?<condition>?
    //   ::tcld::dbg::breakpoint_add file <path> <body_byte_offset> <body_byte_len> ?<condition>?
    //   ::tcld::dbg::breakpoint_add instruction <pc> ?<condition>?
    if !(3..=6).contains(&objc) {
        return set_error(
            interp,
            "wrong # args: should be \"::tcld::dbg::breakpoint_add ?source? proc offset len ?condition?\" or \"::tcld::dbg::breakpoint_add instruction pc ?condition?\"",
        );
    }
    let first = match unsafe { objv_to_string(objv, 1) } {
        Some(s) => s,
        None => return set_error(interp, "breakpoint target missing"),
    };
    let Some(state) = step_engine_state(interp) else {
        return set_error(interp, "step engine not installed");
    };
    let mut guard = match state.lock() {
        Ok(g) => g,
        Err(_) => return set_error(interp, "state mutex poisoned"),
    };

    let bp_id = if first == "instruction" {
        if !(objc == 3 || objc == 4) {
            return set_error(
                interp,
                "wrong # args: should be \"::tcld::dbg::breakpoint_add instruction pc ?condition?\"",
            );
        }
        let pc = match unsafe { objv_to_int(objv, 2) } {
            Some(n) if n >= 0 => n as usize,
            _ => return set_error(interp, "pc must be a non-negative integer"),
        };
        let condition = if objc == 4 {
            unsafe { objv_to_string(objv, 3) }
        } else {
            None
        };
        let block_id = BlockId::ProcBody(Arc::from("<instruction>"));
        if !guard.blocks.contains(&block_id) {
            let _ = guard
                .blocks
                .register(block_id.clone(), Arc::from(" "), vec![0]);
        }
        let location = match Location::new(block_id, 0, 0..1) {
            Ok(l) => l,
            Err(err) => return set_error(interp, &err.to_string()),
        };
        guard.with_breakpoints_mut(|store: &mut BreakpointStore, registry| {
            store.add(
                BreakpointKind::Instruction { pc },
                location,
                condition.clone(),
                false,
                registry,
            )
        })
    } else {
        let (base, block_id) = if first == "source" || first == "file" {
            if !(objc == 5 || objc == 6) {
                return set_error(
                    interp,
                    "wrong # args: should be \"::tcld::dbg::breakpoint_add source|file identity offset len ?condition?\"",
                );
            }
            let identity = match unsafe { objv_to_string(objv, 2) } {
                Some(s) => s,
                None => return set_error(interp, "source identity missing"),
            };
            let block = if first == "file" {
                BlockId::File(Arc::from(identity))
            } else {
                BlockId::ProcBody(Arc::from(identity))
            };
            (2, block)
        } else {
            if !(objc == 4 || objc == 5) {
                return set_error(
                    interp,
                    "wrong # args: should be \"::tcld::dbg::breakpoint_add proc offset len ?condition?\"",
                );
            }
            (1, BlockId::ProcBody(Arc::from(first)))
        };
        let offset = match unsafe { objv_to_int(objv, base + 1) } {
            Some(n) => n,
            None => return set_error(interp, "offset must be an integer"),
        };
        let len = match unsafe { objv_to_int(objv, base + 2) } {
            Some(n) => n,
            None => return set_error(interp, "len must be an integer"),
        };
        let condition = if objc as usize == base + 4 {
            unsafe { objv_to_string(objv, base + 3) }
        } else {
            None
        };
        // Auto-register a placeholder block if none exists. The host can
        // override later with the real source via Block::register.
        if !guard.blocks.contains(&block_id) {
            let placeholder: Arc<str> =
                Arc::from(" ".repeat(offset.max(0) as usize + len.max(1) as usize));
            let _ =
                guard
                    .blocks
                    .register(block_id.clone(), placeholder, vec![offset.max(0) as usize]);
        }

        let location = match Location::new(
            block_id.clone(),
            0,
            offset.max(0) as usize..(offset.max(0) as usize + len.max(1) as usize),
        ) {
            Ok(l) => l,
            Err(err) => return set_error(interp, &err.to_string()),
        };

        guard.with_breakpoints_mut(|store: &mut BreakpointStore, registry| {
            store.add(
                BreakpointKind::Line,
                location,
                condition.clone(),
                false,
                registry,
            )
        })
    };
    drop(guard);

    match bp_id {
        Ok(id) => {
            if let Err(err) = resync_opcode_arm(interp) {
                return set_error(interp, &err);
            }
            set_result(interp, &format!("bp{}", id.0))
        }
        Err(err) => set_error(interp, &err.to_string()),
    }
}

unsafe extern "C" fn file_line_location_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    if objc != 3 {
        return set_error(
            interp,
            "wrong # args: should be \"::tcld::dbg::file_line_location path line\"",
        );
    }
    let path = match unsafe { objv_to_string(objv, 1) } {
        Some(path) => path,
        None => return set_error(interp, "path missing"),
    };
    let line = match unsafe { objv_to_int(objv, 2) } {
        Some(line) if line > 0 => line as u32,
        _ => return set_error(interp, "line must be a positive integer"),
    };
    let Some(state) = step_engine_state(interp) else {
        return set_error(interp, "step engine not installed");
    };
    let block_id = BlockId::File(Arc::from(path.as_str()));
    let offset = match state.lock() {
        Ok(guard) => guard
            .blocks
            .get(&block_id)
            .and_then(|block| block.offset_for_line(line)),
        Err(_) => return set_error(interp, "state mutex poisoned"),
    };
    match offset {
        Some(offset) => set_result(interp, &format!("offset {offset} len 1 line {line}")),
        None => set_error(interp, &format!("no executable command at {path}:{line}")),
    }
}

unsafe extern "C" fn breakpoint_list_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    _objc: c_int,
    _objv: *const *mut Tcl_Obj,
) -> c_int {
    let Some(state) = step_engine_state(interp) else {
        return set_error(interp, "step engine not installed");
    };
    let guard = match state.lock() {
        Ok(g) => g,
        Err(_) => return set_error(interp, "state mutex poisoned"),
    };
    let mut parts = Vec::new();
    for bp in guard.breakpoints().list() {
        let block_token = bp.location.block().as_string();
        let offset = bp.location.byte_offset();
        let line = guard.blocks.get(bp.location.block()).and_then(|block| {
            block
                .line_to_offset()
                .iter()
                .find(|(_, command_offset)| *command_offset == offset)
                .map(|(line, _)| *line)
        });
        let line_field = line.map(|line| format!(" line {line}")).unwrap_or_default();
        let condition_field = bp
            .condition
            .as_deref()
            .filter(|condition| !condition.is_empty())
            .map(|condition| format!(" condition {{{}}}", tcl_escape_braces(condition)))
            .unwrap_or_default();
        parts.push(format!(
            "{{id bp{} kind {} block {{{}}} offset {} len {}{} enabled {} hits {} ignore {} pending 0{}}}",
            bp.id.0,
            match &bp.kind {
                BreakpointKind::Line => "line",
                BreakpointKind::Instruction { .. } => "instruction",
                BreakpointKind::Var { .. } => "var",
                BreakpointKind::Exception { .. } => "exception",
            },
            tcl_escape_braces(&block_token),
            offset,
            bp.location.byte_len(),
            line_field,
            if bp.enabled { 1 } else { 0 },
            bp.hit_count,
            bp.ignore_count,
            condition_field,
        ));
    }
    set_result(interp, &parts.join(" "))
}

unsafe extern "C" fn app_state_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    _objc: c_int,
    _objv: *const *mut Tcl_Obj,
) -> c_int {
    let Some(state) = step_engine_state(interp) else {
        return set_result(interp, "dead");
    };
    let guard = match state.lock() {
        Ok(g) => g,
        Err(_) => return set_error(interp, "state mutex poisoned"),
    };
    let s = match guard.app_state {
        tcld_debug_state::AppState::Dead => "dead",
        tcld_debug_state::AppState::Running => "running",
        tcld_debug_state::AppState::Stopped => "stopped",
    };
    set_result(interp, s)
}

unsafe fn objv_to_int(objv: *const *mut Tcl_Obj, index: usize) -> Option<i64> {
    let s = unsafe { objv_to_string(objv, index) }?;
    s.parse::<i64>().ok()
}

unsafe extern "C" fn install_command_wrappers_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    _objc: c_int,
    _objv: *const *mut Tcl_Obj,
) -> c_int {
    if let Err(err) = unsafe { watchpoints::install_var_trace_helper(interp) } {
        return set_error(interp, &err);
    }
    match unsafe { wrappers::install_command_wrappers(interp) } {
        Ok(()) => set_result(interp, "ok"),
        Err(err) => set_error(interp, &err),
    }
}

unsafe extern "C" fn coverage_enable_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    if objc != 2 {
        return set_error(
            interp,
            "wrong # args: should be \"::tcld::dbg::coverage_enable on|off\"",
        );
    }
    let arg = unsafe { objv_to_string(objv, 1) }.unwrap_or_default();
    let on = matches!(arg.as_str(), "on" | "1" | "true" | "yes");
    unsafe { step_engine::set_coverage_enabled(interp, on) };
    set_result(interp, if on { "on" } else { "off" })
}

unsafe extern "C" fn coverage_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    _objc: c_int,
    _objv: *const *mut Tcl_Obj,
) -> c_int {
    let Some(state) = step_engine_state(interp) else {
        return set_error(interp, "step engine not installed");
    };
    let guard = match state.lock() {
        Ok(g) => g,
        Err(_) => return set_error(interp, "state mutex poisoned"),
    };
    let mut parts = Vec::new();
    for (block, offset, count) in guard.coverage.entries() {
        let key = match block {
            BlockId::ProcBody(name) => format!("proc:{name}"),
            BlockId::File(path) => format!("file:{path}"),
            BlockId::Anonymous { content_hash } => format!("anon:{content_hash:016x}"),
        };
        parts.push(format!("{{block {{{key}}} offset {offset} count {count}}}"));
    }
    set_result(interp, &parts.join(" "))
}

unsafe extern "C" fn watchpoint_add_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    // Usage: ::tcld::dbg::watchpoint_add <var_name> <ops>
    // ops is a subset of {read write unset} (space-separated).
    if objc != 3 {
        return set_error(
            interp,
            "wrong # args: should be \"::tcld::dbg::watchpoint_add var ops\"",
        );
    }
    let name = match unsafe { objv_to_string(objv, 1) } {
        Some(s) => s,
        None => return set_error(interp, "var name missing"),
    };
    let ops_text = unsafe { objv_to_string(objv, 2) }.unwrap_or_default();
    let on_read = ops_text.contains("read");
    let on_write = ops_text.contains("write");
    let on_unset = ops_text.contains("unset");
    if !(on_read || on_write || on_unset) {
        return set_error(interp, "ops must include at least one of read/write/unset");
    }

    let Some(state) = step_engine_state(interp) else {
        return set_error(interp, "step engine not installed");
    };
    let mut guard = match state.lock() {
        Ok(g) => g,
        Err(_) => return set_error(interp, "state mutex poisoned"),
    };

    // Anonymous Block as anchor — var bps don't have a source location.
    let block_id = BlockId::ProcBody(Arc::from(format!("__var_anchor_{name}__")));
    if !guard.blocks.contains(&block_id) {
        let _ = guard
            .blocks
            .register(block_id.clone(), Arc::from("var"), vec![0]);
    }
    let location = match Location::new(block_id, 0, 0..1) {
        Ok(l) => l,
        Err(err) => return set_error(interp, &err.to_string()),
    };
    let bp_id = guard.with_breakpoints_mut(|store, registry| {
        store.add(
            BreakpointKind::Var {
                name: name.clone(),
                on_read,
                on_write,
                on_unset,
            },
            location,
            None,
            false,
            registry,
        )
    });
    let bp_id = match bp_id {
        Ok(id) => id,
        Err(err) => return set_error(interp, &err.to_string()),
    };
    drop(guard);

    if let Err(err) =
        unsafe { watchpoints::arm_watchpoint(interp, &name, on_read, on_write, on_unset, bp_id) }
    {
        return set_error(interp, &err);
    }
    set_result(interp, &format!("bp{}", bp_id.0))
}

unsafe extern "C" fn inspect_stack_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    _objc: c_int,
    _objv: *const *mut Tcl_Obj,
) -> c_int {
    // Stack inspection is valid only while the user's frame is suspended.
    if let Some(state) = step_engine_state(interp) {
        let guard = match state.lock() {
            Ok(g) => g,
            Err(_) => return set_error(interp, "state mutex poisoned"),
        };
        if let Err(err) = guard.require_stopped() {
            return set_error(interp, &err.to_string());
        }
    }

    let frames = unsafe { step_engine::inspect_stack(interp) };
    let rows = frames
        .into_iter()
        .map(|frame| {
            let mut fields = vec![
                "level".to_string(),
                frame.level.to_string(),
                "kind".to_string(),
                frame.kind.as_str().to_string(),
            ];
            if let Some(qname) = &frame.proc_qname {
                fields.extend(["proc".to_string(), qname.to_string()]);
            }
            if let Some(path) = &frame.source_path {
                fields.extend(["source_path".to_string(), path.to_string()]);
            }
            if let Some(location) = &frame.location {
                let (block_kind, qualified_proc) = match location.block() {
                    tcld_debug_state::BlockId::File(_) => ("file", None),
                    tcld_debug_state::BlockId::ProcBody(qname) => ("proc", Some(qname.to_string())),
                    tcld_debug_state::BlockId::Anonymous { .. } => ("anonymous", None),
                };
                fields.extend([
                    "block".to_string(),
                    location.block().as_string(),
                    "block_kind".to_string(),
                    block_kind.to_string(),
                ]);
                if let Some(qname) = qualified_proc {
                    fields.extend(["qualified_proc".to_string(), qname]);
                }
                fields.extend([
                    "line".to_string(),
                    location.line().to_string(),
                    "offset_start".to_string(),
                    location.byte_offset().to_string(),
                    "offset_end".to_string(),
                    location
                        .byte_offset()
                        .saturating_add(location.byte_len())
                        .to_string(),
                ]);
            }
            tcld_debug_wire::tcl_list_format(fields.iter())
        })
        .collect::<Vec<_>>();
    set_result(interp, &tcld_debug_wire::tcl_list_format(rows.iter()))
}

fn simple_dict_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        if chars[i] == '{' {
            let mut depth = 1;
            let mut j = i + 1;
            while j < chars.len() && depth > 0 {
                match chars[j] {
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    _ => {}
                }
                if depth > 0 {
                    j += 1;
                }
            }
            tokens.push(chars[i + 1..j].iter().collect());
            i = j + 1;
        } else {
            let start = i;
            while i < chars.len() && !chars[i].is_whitespace() {
                i += 1;
            }
            tokens.push(chars[start..i].iter().collect());
        }
    }
    tokens
}

unsafe extern "C" fn eval_in_frame_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    // Usage: ::tcld::dbg::eval_in_frame <level> <script>
    if objc != 3 {
        return set_error(
            interp,
            "wrong # args: should be \"::tcld::dbg::eval_in_frame level script\"",
        );
    }
    let level = match unsafe { objv_to_int(objv, 1) } {
        Some(n) => n,
        None => return set_error(interp, "level must be an integer"),
    };
    let script = match unsafe { objv_to_string(objv, 2) } {
        Some(s) => s,
        None => return set_error(interp, "script missing"),
    };

    // Evaluating in a *paused frame* (`uplevel #N`, N>0) requires a
    // stop — that frame only exists mid-pause. The global frame (`#0`),
    // however, always exists, so a level-0 eval is valid at any time,
    // including at idle before the first pause. This is what lets the
    // controller read startup globals such as the nub's load status.
    if level > 0
        && let Some(state) = step_engine_state(interp)
    {
        let guard = match state.lock() {
            Ok(g) => g,
            Err(_) => return set_error(interp, "state mutex poisoned"),
        };
        if let Err(err) = guard.require_stopped() {
            return set_error(interp, &err.to_string());
        }
    }

    // Wrap in `uplevel #$level { ... }` so the script runs in the
    // requested frame's scope. `#0` == global, `#$N` == proc level N.
    let wrapped = if level <= 0 {
        format!("uplevel #0 {{{}}}", script)
    } else {
        format!("uplevel #{} {{{}}}", level, script)
    };
    let cstr = match CString::new(wrapped) {
        Ok(c) => c,
        Err(_) => return set_error(interp, "script contains NUL byte"),
    };
    unsafe { Tcl_EvalEx(interp, cstr.as_ptr(), -1, 0) }
}

// ─── Tier 2 commands ─────────────────────────────────────────────────
//
// Variable inspection at frame, proc registry, last-result capture,
// hit-count gating, error-ignore, and breakpoint lifecycle ops. These
// close the gap between TclPro's `dbg::*` surface and tcld's existing
// Tier 0/1 wiring without introducing a new persistent system (no
// profiler runtime, no slave-interp registration — those are out of
// scope for Tier 2 by judgment, see Tier 2 plan summary).

/// Tcl helper script installed at engine_install time. Provides
/// `__t2_get_variables__`, `__t2_get_var__`, `__t2_set_var__`. Defining
/// these as Tcl procs (rather than building scripts ad-hoc per call)
/// keeps the user's frame uncluttered — only the helper proc's local
/// vars are visible to `info vars` inside the body.
const TIER2_HELPER_SCRIPT: &str = r#"
namespace eval ::tcld::dbg {}

proc ::tcld::dbg::__t2_get_variables__ {level} {
    set vars [uplevel #$level {info vars}]
    set out {}
    foreach n $vars {
        if {[uplevel #$level [list array exists $n]]} {
            lappend out [list $n a]
        } else {
            lappend out [list $n s]
        }
    }
    return $out
}

proc ::tcld::dbg::__t2_get_var__ {level name maxlen} {
    if {[uplevel #$level [list array exists $name]]} {
        set raw [uplevel #$level [list array get $name]]
    } else {
        set raw [uplevel #$level [list set $name]]
    }
    if {$maxlen > 0 && [string length $raw] > $maxlen} {
        return "[string range $raw 0 [expr {$maxlen - 1}]]..."
    }
    return $raw
}

proc ::tcld::dbg::__t2_set_var__ {level name value} {
    return [uplevel #$level [list set $name $value]]
}
"#;

unsafe fn install_tier2_helpers(interp: *mut Tcl_Interp) -> Result<(), String> {
    let cstr =
        CString::new(TIER2_HELPER_SCRIPT).map_err(|_| "tier2 helper script has NUL".to_string())?;
    let rc = unsafe { Tcl_EvalEx(interp, cstr.as_ptr(), -1, 0) };
    if rc != TCL_OK {
        return Err("tier2 helper install failed".to_string());
    }
    Ok(())
}

unsafe fn eval_helper(
    interp: *mut Tcl_Interp,
    helper: &str,
    args: &[String],
) -> Result<String, String> {
    // Build a Tcl-list-quoted call so values containing whitespace pass
    // through without being re-tokenised.
    let mut script = String::from(helper);
    for a in args {
        script.push(' ');
        script.push('{');
        script.push_str(&tcl_escape_braces(a));
        script.push('}');
    }
    let cstr = CString::new(script).map_err(|_| "script NUL".to_string())?;
    let rc = unsafe { Tcl_EvalEx(interp, cstr.as_ptr(), -1, 0) };
    let result = unsafe { result_string(interp) };
    if rc != TCL_OK {
        Err(result)
    } else {
        Ok(result)
    }
}

unsafe fn result_string(interp: *mut Tcl_Interp) -> String {
    let obj = unsafe { Tcl_GetObjResult(interp) };
    if obj.is_null() {
        return String::new();
    }
    let mut len: tcl_sys::Tcl_Size = 0;
    let ptr = unsafe { Tcl_GetStringFromObj(obj, &mut len) };
    if ptr.is_null() || len <= 0 {
        return String::new();
    }
    let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) };
    String::from_utf8_lossy(bytes).into_owned()
}

unsafe extern "C" fn get_variables_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    // Usage: ::tcld::dbg::get_variables <level>
    if objc != 2 {
        return set_error(
            interp,
            "wrong # args: should be \"::tcld::dbg::get_variables level\"",
        );
    }
    let level = match unsafe { objv_to_int(objv, 1) } {
        Some(n) => n.to_string(),
        None => return set_error(interp, "level must be an integer"),
    };
    if let Some(state) = step_engine_state(interp) {
        let guard = match state.lock() {
            Ok(g) => g,
            Err(_) => return set_error(interp, "state mutex poisoned"),
        };
        if let Err(err) = guard.require_stopped() {
            return set_error(interp, &err.to_string());
        }
    }
    match unsafe { eval_helper(interp, "::tcld::dbg::__t2_get_variables__", &[level]) } {
        Ok(text) => set_result(interp, &text),
        Err(err) => set_error(interp, &err),
    }
}

unsafe extern "C" fn get_var_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    // Usage: ::tcld::dbg::get_var <level> <name> ?<maxlen>?
    if !(3..=4).contains(&objc) {
        return set_error(
            interp,
            "wrong # args: should be \"::tcld::dbg::get_var level name ?maxlen?\"",
        );
    }
    let level = match unsafe { objv_to_int(objv, 1) } {
        Some(n) => n.to_string(),
        None => return set_error(interp, "level must be an integer"),
    };
    let name = match unsafe { objv_to_string(objv, 2) } {
        Some(s) => s,
        None => return set_error(interp, "name missing"),
    };
    let maxlen = if objc == 4 {
        match unsafe { objv_to_int(objv, 3) } {
            Some(n) if n >= 0 => n.to_string(),
            _ => return set_error(interp, "maxlen must be a non-negative integer"),
        }
    } else {
        "0".to_string()
    };
    if let Some(state) = step_engine_state(interp) {
        let guard = match state.lock() {
            Ok(g) => g,
            Err(_) => return set_error(interp, "state mutex poisoned"),
        };
        if let Err(err) = guard.require_stopped() {
            return set_error(interp, &err.to_string());
        }
    }
    match unsafe {
        eval_helper(
            interp,
            "::tcld::dbg::__t2_get_var__",
            &[level, name, maxlen],
        )
    } {
        Ok(text) => set_result(interp, &text),
        Err(err) => set_error(interp, &err),
    }
}

unsafe extern "C" fn set_var_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    // Usage: ::tcld::dbg::set_var <level> <name> <value>
    if objc != 4 {
        return set_error(
            interp,
            "wrong # args: should be \"::tcld::dbg::set_var level name value\"",
        );
    }
    let level = match unsafe { objv_to_int(objv, 1) } {
        Some(n) => n.to_string(),
        None => return set_error(interp, "level must be an integer"),
    };
    let name = match unsafe { objv_to_string(objv, 2) } {
        Some(s) => s,
        None => return set_error(interp, "name missing"),
    };
    let value = unsafe { objv_to_string(objv, 3) }.unwrap_or_default();
    if let Some(state) = step_engine_state(interp) {
        let guard = match state.lock() {
            Ok(g) => g,
            Err(_) => return set_error(interp, "state mutex poisoned"),
        };
        if let Err(err) = guard.require_stopped() {
            return set_error(interp, &err.to_string());
        }
    }
    match unsafe { eval_helper(interp, "::tcld::dbg::__t2_set_var__", &[level, name, value]) } {
        Ok(text) => set_result(interp, &text),
        Err(err) => set_error(interp, &err),
    }
}

unsafe extern "C" fn get_result_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    // Usage: ::tcld::dbg::get_result ?<maxlen>?
    // Returns the result the trace handler captured on the most recent
    // ENTER_EXEC — that's the result of the previously completed
    // command. Tcl resets the interp's obj result before each
    // command-dispatch, so reading `Tcl_GetObjResult` from a separate
    // command-eval would always be empty; use the captured snapshot.
    if objc > 2 {
        return set_error(
            interp,
            "wrong # args: should be \"::tcld::dbg::get_result ?maxlen?\"",
        );
    }
    let maxlen = if objc == 2 {
        match unsafe { objv_to_int(objv, 1) } {
            Some(n) if n >= 0 => n as usize,
            _ => return set_error(interp, "maxlen must be a non-negative integer"),
        }
    } else {
        0
    };
    let text = match step_engine_state(interp) {
        Some(state) => match state.lock() {
            Ok(g) => g.last_result.clone().unwrap_or_default(),
            Err(_) => return set_error(interp, "state mutex poisoned"),
        },
        None => unsafe { result_string(interp) },
    };
    let truncated = if maxlen > 0 && text.len() > maxlen {
        let mut s: String = text.chars().take(maxlen).collect();
        s.push_str("...");
        s
    } else {
        text
    };
    set_result(interp, &truncated)
}

unsafe extern "C" fn list_procs_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    _objc: c_int,
    _objv: *const *mut Tcl_Obj,
) -> c_int {
    let Some(state) = step_engine_state(interp) else {
        return set_error(interp, "step engine not installed");
    };
    let guard = match state.lock() {
        Ok(g) => g,
        Err(_) => return set_error(interp, "state mutex poisoned"),
    };
    let mut names: Vec<String> = guard
        .blocks
        .iter()
        .filter_map(|(id, _)| match id {
            BlockId::ProcBody(name) => Some(name.to_string()),
            _ => None,
        })
        .collect();
    names.sort();
    let escaped: Vec<String> = names.iter().map(|n| format!("{{{n}}}")).collect();
    set_result(interp, &escaped.join(" "))
}

unsafe extern "C" fn proc_body_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    if objc != 2 {
        return set_error(
            interp,
            "wrong # args: should be \"::tcld::dbg::proc_body qname\"",
        );
    }
    let qname = match unsafe { objv_to_string(objv, 1) } {
        Some(s) => s,
        None => return set_error(interp, "qname missing"),
    };
    let bare = qname.trim_start_matches("::");
    let Some(state) = step_engine_state(interp) else {
        return set_error(interp, "step engine not installed");
    };
    let guard = match state.lock() {
        Ok(g) => g,
        Err(_) => return set_error(interp, "state mutex poisoned"),
    };
    let id = BlockId::ProcBody(Arc::from(bare));
    match guard.blocks.get(&id) {
        Some(block) => set_result(interp, block.source()),
        None => set_error(interp, &format!("proc body not registered: {qname}")),
    }
}

unsafe extern "C" fn block_source_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    if objc != 2 {
        return set_error(
            interp,
            "wrong # args: should be \"::tcld::dbg::block_source owner_block_id\"",
        );
    }
    let token = match unsafe { objv_to_string(objv, 1) } {
        Some(token) => token,
        None => return set_error(interp, "owner block id missing"),
    };
    let id = if let Some(path) = token.strip_prefix("file:") {
        BlockId::File(Arc::from(path))
    } else if let Some(qname) = token.strip_prefix("proc:") {
        BlockId::ProcBody(Arc::from(qname.trim_start_matches("::")))
    } else if let Some(hash) = token.strip_prefix("anon:") {
        match u64::from_str_radix(hash, 16) {
            Ok(content_hash) => BlockId::Anonymous { content_hash },
            Err(_) => return set_error(interp, "invalid anonymous owner block id"),
        }
    } else {
        return set_error(interp, "unknown owner block id kind");
    };
    let Some(state) = step_engine_state(interp) else {
        return set_error(interp, "step engine not installed");
    };
    let guard = match state.lock() {
        Ok(guard) => guard,
        Err(_) => return set_error(interp, "state mutex poisoned"),
    };
    match guard.blocks.get(&id) {
        Some(block) => set_result(interp, block.source()),
        None => set_error(interp, &format!("source unit unavailable: {token}")),
    }
}

unsafe extern "C" fn block_generation_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    if objc != 2 {
        return set_error(
            interp,
            "wrong # args: should be \"::tcld::dbg::block_generation owner_block_id\"",
        );
    }
    let token = match unsafe { objv_to_string(objv, 1) } {
        Some(token) => token,
        None => return set_error(interp, "owner block id missing"),
    };
    let id = if let Some(path) = token.strip_prefix("file:") {
        BlockId::File(Arc::from(path))
    } else if let Some(qname) = token.strip_prefix("proc:") {
        BlockId::ProcBody(Arc::from(qname.trim_start_matches("::")))
    } else if let Some(hash) = token.strip_prefix("anon:") {
        match u64::from_str_radix(hash, 16) {
            Ok(content_hash) => BlockId::Anonymous { content_hash },
            Err(_) => return set_error(interp, "invalid anonymous owner block id"),
        }
    } else {
        return set_error(interp, "unknown owner block id kind");
    };
    let Some(state) = step_engine_state(interp) else {
        return set_error(interp, "step engine not installed");
    };
    let guard = match state.lock() {
        Ok(guard) => guard,
        Err(_) => return set_error(interp, "state mutex poisoned"),
    };
    match guard.blocks.get(&id) {
        Some(block) => set_result(interp, &block.version().to_string()),
        None => set_error(interp, &format!("source unit unavailable: {token}")),
    }
}

unsafe extern "C" fn block_command_span_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    if objc != 4 {
        return set_error(
            interp,
            "wrong # args: block_command_span owner_block_id offset requested_len",
        );
    }
    let Some(token) = (unsafe { objv_to_string(objv, 1) }) else {
        return set_error(interp, "owner block id missing");
    };
    let Some(offset) =
        (unsafe { objv_to_int(objv, 2) }).and_then(|value| usize::try_from(value).ok())
    else {
        return set_error(interp, "command offset must be non-negative");
    };
    let Some(requested_len) =
        (unsafe { objv_to_int(objv, 3) }).and_then(|value| usize::try_from(value).ok())
    else {
        return set_error(interp, "command length must be non-negative");
    };
    let id = if let Some(path) = token.strip_prefix("file:") {
        BlockId::File(Arc::from(path))
    } else if let Some(qname) = token.strip_prefix("proc:") {
        BlockId::ProcBody(Arc::from(qname.trim_start_matches("::")))
    } else {
        return set_error(interp, "command span requires a file or proc owner");
    };
    let Some(state) = step_engine_state(interp) else {
        return set_error(interp, "step engine not installed");
    };
    let guard = match state.lock() {
        Ok(guard) => guard,
        Err(_) => return set_error(interp, "state mutex poisoned"),
    };
    let Some(block) = guard.blocks.get(&id) else {
        return set_error(interp, &format!("source unit unavailable: {token}"));
    };
    let parse_source;
    let source = if block.source().ends_with('\n') {
        block.source()
    } else {
        parse_source = format!("{}\n", block.source());
        &parse_source
    };
    let mut candidates: Vec<_> = tcld_syntax::command_spans(source)
        .unwrap_or_default()
        .into_iter()
        .filter(|span| span.start_byte == offset && span.end_byte > span.start_byte)
        .collect();
    if candidates.is_empty() {
        return set_error(
            interp,
            &format!("no executable command starts at {token}+{offset}"),
        );
    }
    candidates.sort_by_key(|span| {
        let len = span.end_byte - span.start_byte;
        (usize::from(len != requested_len), len)
    });
    let span = &candidates[0];
    let text = &source[span.start_byte..span.end_byte];
    let payload = tcld_debug_wire::tcl_list_format([
        "offset".to_string(),
        span.start_byte.to_string(),
        "len".to_string(),
        (span.end_byte - span.start_byte).to_string(),
        "text".to_string(),
        text.to_string(),
    ]);
    set_result(interp, &payload)
}

unsafe extern "C" fn bind_logical_breakpoint_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    if objc != 3 {
        return set_error(
            interp,
            "wrong # args: __bind_logical_breakpoint logical_id core_id",
        );
    }
    let Some(logical_id) = (unsafe { objv_to_string(objv, 1) }) else {
        return set_error(interp, "logical breakpoint id missing");
    };
    let Some(core_id) = (unsafe { objv_to_string(objv, 2) }) else {
        return set_error(interp, "core breakpoint id missing");
    };
    step_engine::bind_logical_breakpoint(interp, logical_id, core_id);
    set_result(interp, "ok")
}

unsafe extern "C" fn unbind_logical_breakpoint_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    if objc != 2 {
        return set_error(interp, "wrong # args: __unbind_logical_breakpoint core_id");
    }
    let Some(core_id) = (unsafe { objv_to_string(objv, 1) }) else {
        return set_error(interp, "core breakpoint id missing");
    };
    step_engine::unbind_logical_breakpoint(interp, &core_id);
    set_result(interp, "ok")
}

unsafe extern "C" fn proc_location_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    if objc != 2 {
        return set_error(
            interp,
            "wrong # args: should be \"::tcld::dbg::proc_location qname\"",
        );
    }
    let qname = match unsafe { objv_to_string(objv, 1) } {
        Some(s) => s,
        None => return set_error(interp, "qname missing"),
    };
    let bare = qname.trim_start_matches("::");
    let Some(state) = step_engine_state(interp) else {
        return set_error(interp, "step engine not installed");
    };
    let guard = match state.lock() {
        Ok(g) => g,
        Err(_) => return set_error(interp, "state mutex poisoned"),
    };
    let id = BlockId::ProcBody(Arc::from(bare));
    match guard.blocks.get(&id) {
        Some(block) => {
            let first = block.valid_offsets().first().copied().unwrap_or(0);
            let dict = format!(
                "block {{proc:{bare}}} first_offset {first} version {} cmd_count {}",
                block.version(),
                block.valid_offsets().len()
            );
            set_result(interp, &dict)
        }
        None => set_error(interp, &format!("proc body not registered: {qname}")),
    }
}

unsafe extern "C" fn block_disable_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    block_set_disabled(interp, objc, objv, true)
}

unsafe extern "C" fn block_enable_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    block_set_disabled(interp, objc, objv, false)
}

fn block_set_disabled(
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
    disable: bool,
) -> c_int {
    if objc != 2 {
        return set_error(
            interp,
            "wrong # args: should be \"qname\" (proc qname; file: paths and anon: hashes are not supported here)",
        );
    }
    let qname = match unsafe { objv_to_string(objv, 1) } {
        Some(s) => s,
        None => return set_error(interp, "qname missing"),
    };
    let bare = qname.trim_start_matches("::");
    let Some(state) = step_engine_state(interp) else {
        return set_error(interp, "step engine not installed");
    };
    let mut guard = match state.lock() {
        Ok(g) => g,
        Err(_) => return set_error(interp, "state mutex poisoned"),
    };
    let id = BlockId::ProcBody(Arc::from(bare));
    if disable {
        guard.disable_block(id);
    } else {
        guard.enable_block(&id);
    }
    set_result(interp, "ok")
}

unsafe extern "C" fn ignore_error_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    _objc: c_int,
    _objv: *const *mut Tcl_Obj,
) -> c_int {
    let Some(state) = step_engine_state(interp) else {
        return set_error(interp, "step engine not installed");
    };
    let mut guard = match state.lock() {
        Ok(g) => g,
        Err(_) => return set_error(interp, "state mutex poisoned"),
    };
    guard.error_ignored_pending = true;
    set_result(interp, "ok")
}

unsafe extern "C" fn bp_remove_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    let id = match unsafe { parse_bp_id_arg(interp, objc, objv) } {
        Ok(id) => id,
        Err(rc) => return rc,
    };
    let Some(state) = step_engine_state(interp) else {
        return set_error(interp, "step engine not installed");
    };
    let mut guard = match state.lock() {
        Ok(g) => g,
        Err(_) => return set_error(interp, "state mutex poisoned"),
    };
    let result = guard.with_breakpoints_mut(|store, _| store.remove(id));
    drop(guard);
    match result {
        Ok(_) => {
            if let Err(err) = resync_opcode_arm(interp) {
                return set_error(interp, &err);
            }
            set_result(interp, "ok")
        }
        Err(err) => set_error(interp, &err.to_string()),
    }
}

unsafe extern "C" fn bp_disable_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    bp_set_enabled(interp, objc, objv, false)
}

unsafe extern "C" fn bp_enable_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    bp_set_enabled(interp, objc, objv, true)
}

fn bp_set_enabled(
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
    enable: bool,
) -> c_int {
    let id = match unsafe { parse_bp_id_arg(interp, objc, objv) } {
        Ok(id) => id,
        Err(rc) => return rc,
    };
    let Some(state) = step_engine_state(interp) else {
        return set_error(interp, "step engine not installed");
    };
    let mut guard = match state.lock() {
        Ok(g) => g,
        Err(_) => return set_error(interp, "state mutex poisoned"),
    };
    let result = guard.with_breakpoints_mut(|store, _| {
        if enable {
            store.enable(id)
        } else {
            store.disable(id)
        }
    });
    drop(guard);
    match result {
        Ok(()) => {
            if let Err(err) = resync_opcode_arm(interp) {
                return set_error(interp, &err);
            }
            set_result(interp, "ok")
        }
        Err(err) => set_error(interp, &err.to_string()),
    }
}

unsafe extern "C" fn bp_set_ignore_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    // Usage: ::tcld::dbg::bp_set_ignore <bp_id> <count>
    if objc != 3 {
        return set_error(
            interp,
            "wrong # args: should be \"::tcld::dbg::bp_set_ignore bp_id count\"",
        );
    }
    let id = match unsafe { parse_bp_id_token(objv, 1) } {
        Some(id) => id,
        None => return set_error(interp, "bp_id must be in form bp<N>"),
    };
    let count = match unsafe { objv_to_int(objv, 2) } {
        Some(n) if n >= 0 => n as u64,
        _ => return set_error(interp, "count must be a non-negative integer"),
    };
    let Some(state) = step_engine_state(interp) else {
        return set_error(interp, "step engine not installed");
    };
    let mut guard = match state.lock() {
        Ok(g) => g,
        Err(_) => return set_error(interp, "state mutex poisoned"),
    };
    let result = guard.with_breakpoints_mut(|store, _| store.set_ignore_count(id, count));
    match result {
        Ok(()) => set_result(interp, "ok"),
        Err(err) => set_error(interp, &err.to_string()),
    }
}

unsafe extern "C" fn bp_move_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    // Usage: ::tcld::dbg::bp_move <bp_id> <qname> <offset> <len>
    if objc != 5 {
        return set_error(
            interp,
            "wrong # args: should be \"::tcld::dbg::bp_move bp_id qname offset len\"",
        );
    }
    let id = match unsafe { parse_bp_id_token(objv, 1) } {
        Some(id) => id,
        None => return set_error(interp, "bp_id must be in form bp<N>"),
    };
    let qname = match unsafe { objv_to_string(objv, 2) } {
        Some(s) => s,
        None => return set_error(interp, "qname missing"),
    };
    let offset = match unsafe { objv_to_int(objv, 3) } {
        Some(n) if n >= 0 => n as usize,
        _ => return set_error(interp, "offset must be non-negative integer"),
    };
    let len = match unsafe { objv_to_int(objv, 4) } {
        Some(n) if n >= 1 => n as usize,
        _ => return set_error(interp, "len must be positive integer"),
    };
    let bare = qname.trim_start_matches("::");
    let block_id = BlockId::ProcBody(Arc::from(bare));
    let new_loc = match Location::new(block_id, 0, offset..offset + len) {
        Ok(l) => l,
        Err(err) => return set_error(interp, &err.to_string()),
    };
    let Some(state) = step_engine_state(interp) else {
        return set_error(interp, "step engine not installed");
    };
    let mut guard = match state.lock() {
        Ok(g) => g,
        Err(_) => return set_error(interp, "state mutex poisoned"),
    };
    let result =
        guard.with_breakpoints_mut(|store, registry| store.relocate(id, new_loc, registry));
    match result {
        Ok(()) => set_result(interp, "ok"),
        Err(err) => set_error(interp, &err.to_string()),
    }
}

unsafe extern "C" fn bp_validate_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    // Usage: ::tcld::dbg::bp_validate <qname>
    // Returns a list of {bp_id outcome ?was? ?now?} per breakpoint in
    // the named block. `outcome` is unchanged | snapped | disabled.
    if objc != 2 {
        return set_error(
            interp,
            "wrong # args: should be \"::tcld::dbg::bp_validate qname\"",
        );
    }
    let qname = match unsafe { objv_to_string(objv, 1) } {
        Some(s) => s,
        None => return set_error(interp, "qname missing"),
    };
    let bare = qname.trim_start_matches("::");
    let Some(state) = step_engine_state(interp) else {
        return set_error(interp, "step engine not installed");
    };
    let mut guard = match state.lock() {
        Ok(g) => g,
        Err(_) => return set_error(interp, "state mutex poisoned"),
    };
    let id = BlockId::ProcBody(Arc::from(bare));
    let block = match guard.blocks.get(&id).cloned() {
        Some(b) => b,
        None => return set_error(interp, &format!("block not registered: {qname}")),
    };
    let outcomes = guard.with_breakpoints_mut(|store, _| store.validate_against(&block));
    let parts: Vec<String> = outcomes
        .into_iter()
        .map(|(bp_id, outcome)| match outcome {
            tcld_debug_state::ValidationOutcome::Unchanged => {
                format!("{{id bp{} outcome unchanged}}", bp_id.0)
            }
            tcld_debug_state::ValidationOutcome::Snapped { was, now } => {
                format!("{{id bp{} outcome snapped was {was} now {now}}}", bp_id.0)
            }
            tcld_debug_state::ValidationOutcome::Disabled { was } => {
                format!("{{id bp{} outcome disabled was {was}}}", bp_id.0)
            }
        })
        .collect();
    set_result(interp, &parts.join(" "))
}

unsafe fn parse_bp_id_arg(
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> Result<tcld_debug_state::BreakpointId, c_int> {
    if objc != 2 {
        return Err(set_error(
            interp,
            "wrong # args: should be \"<command> bp_id\"",
        ));
    }
    unsafe { parse_bp_id_token(objv, 1) }
        .ok_or_else(|| set_error(interp, "bp_id must be in form bp<N>"))
}

unsafe fn parse_bp_id_token(
    objv: *const *mut Tcl_Obj,
    index: usize,
) -> Option<tcld_debug_state::BreakpointId> {
    let s = unsafe { objv_to_string(objv, index) }?;
    let n = s.strip_prefix("bp").and_then(|t| t.parse::<u64>().ok())?;
    Some(tcld_debug_state::BreakpointId(n))
}

// ─── Tier 3 commands ─────────────────────────────────────────────────
//
// Async pause, run-to-cursor, watch expressions, persisted bp store,
// slave-interp ops. Builds on Tier 2's lifecycle commands and the
// state crate's WatchStore + pause_requested + slaves additions.

unsafe extern "C" fn request_pause_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    _objc: c_int,
    _objv: *const *mut Tcl_Obj,
) -> c_int {
    if !step_engine::request_pause(interp) {
        return set_error(interp, "step engine not installed");
    }
    set_result(interp, "ok")
}

unsafe extern "C" fn activate_target_file_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    if objc != 3 {
        return set_error(
            interp,
            "wrong # args: should be \"::tcld::dbg::activate_target_file path stop_on_entry\"",
        );
    }
    let path = match unsafe { objv_to_string(objv, 1) } {
        Some(path) => path,
        None => return set_error(interp, "path missing"),
    };
    let Some(state) = step_engine_state(interp) else {
        return set_error(interp, "step engine not installed");
    };
    let block_id = BlockId::File(Arc::from(path.as_str()));
    let stop_on_entry = unsafe { objv_to_string(objv, 2) }
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "on" | "yes"));
    let offset = match state.lock() {
        Ok(guard) => guard
            .blocks
            .get(&block_id)
            .and_then(|block| block.valid_offsets().first().copied()),
        Err(_) => return set_error(interp, "state mutex poisoned"),
    };
    if let Err(error) = step_engine::activate_target_file(
        interp,
        block_id,
        stop_on_entry.then_some(offset).flatten(),
    ) {
        return set_error(interp, &error);
    }
    match (stop_on_entry, offset) {
        (true, Some(offset)) => set_result(interp, &format!("armed {offset}")),
        (true, None) => set_result(interp, "empty"),
        (false, _) => set_result(interp, "active"),
    }
}

unsafe extern "C" fn clear_active_target_file_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    _objc: c_int,
    _objv: *const *mut Tcl_Obj,
) -> c_int {
    step_engine::clear_active_target_file(interp);
    set_result(interp, "ok")
}

unsafe extern "C" fn pause_at_target_entry_cmd(
    _client_data: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    if objc != 2 {
        return set_error(
            interp,
            "wrong # args: should be \"::tcld::dbg::pause_at_target_entry path\"",
        );
    }
    let path = match unsafe { objv_to_string(objv, 1) } {
        Some(path) => path,
        None => return set_error(interp, "path missing"),
    };
    let Some(state) = step_engine_state(interp) else {
        return set_error(interp, "step engine not installed");
    };
    let block_id = BlockId::File(Arc::from(path.as_str()));
    let location = match state.lock() {
        Ok(guard) => guard.blocks.get(&block_id).and_then(|block| {
            let offset = block.valid_offsets().first().copied()?;
            let line = block
                .line_to_offset()
                .iter()
                .find_map(|(line, candidate)| (*candidate == offset).then_some(*line as usize))
                .unwrap_or(1);
            Location::new(block_id.clone(), line, offset..offset.saturating_add(1)).ok()
        }),
        Err(_) => return set_error(interp, "state mutex poisoned"),
    };
    let Some(location) = location else {
        return set_result(interp, "empty");
    };
    match unsafe { step_engine::pause_at_target_entry(interp, location) } {
        Ok(()) => set_result(interp, "resumed"),
        Err(error) => set_error(interp, &error),
    }
}

unsafe extern "C" fn run_to_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    // Usage: ::tcld::dbg::run_to <qname> <offset> <len>
    // Adds a temporary line breakpoint at the offset and resumes from
    // a paused state. The breakpoint auto-removes on first hit.
    if objc != 4 {
        return set_error(
            interp,
            "wrong # args: should be \"::tcld::dbg::run_to qname offset len\"",
        );
    }
    let qname = match unsafe { objv_to_string(objv, 1) } {
        Some(s) => s,
        None => return set_error(interp, "qname missing"),
    };
    let offset = match unsafe { objv_to_int(objv, 2) } {
        Some(n) if n >= 0 => n as usize,
        _ => return set_error(interp, "offset must be non-negative integer"),
    };
    let len = match unsafe { objv_to_int(objv, 3) } {
        Some(n) if n >= 1 => n as usize,
        _ => return set_error(interp, "len must be positive integer"),
    };
    let bare = qname.trim_start_matches("::");
    let block_id = BlockId::ProcBody(Arc::from(bare));
    let location = match Location::new(block_id.clone(), 0, offset..offset + len) {
        Ok(l) => l,
        Err(err) => return set_error(interp, &err.to_string()),
    };
    let Some(state) = step_engine_state(interp) else {
        return set_error(interp, "step engine not installed");
    };
    let bp_id = {
        let mut guard = match state.lock() {
            Ok(g) => g,
            Err(_) => return set_error(interp, "state mutex poisoned"),
        };
        guard.with_breakpoints_mut(|store, registry| {
            store.add(BreakpointKind::Line, location, None, true, registry)
        })
    };
    let bp_id = match bp_id {
        Ok(id) => id,
        Err(err) => return set_error(interp, &err.to_string()),
    };
    // Resume — same path as the `continue` command.
    if let Err(err) =
        unsafe { step_engine::set_step_mode(interp, step_engine::StepMode::Continue, 0) }
    {
        return set_error(interp, &err);
    }
    if let Err(err) = sync_opcode_arm_for_non_opcode_resume(interp) {
        return set_error(interp, &err);
    }
    unsafe { step_engine::signal_resume(interp) };
    set_result(interp, &format!("bp{}", bp_id.0))
}

unsafe extern "C" fn watch_add_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    // Usage: ::tcld::dbg::watch_add <name> <expr>
    if objc != 3 {
        return set_error(
            interp,
            "wrong # args: should be \"::tcld::dbg::watch_add name expr\"",
        );
    }
    let name = match unsafe { objv_to_string(objv, 1) } {
        Some(s) => s,
        None => return set_error(interp, "name missing"),
    };
    let expr = unsafe { objv_to_string(objv, 2) }.unwrap_or_default();
    let Some(state) = step_engine_state(interp) else {
        return set_error(interp, "step engine not installed");
    };
    let mut guard = match state.lock() {
        Ok(g) => g,
        Err(_) => return set_error(interp, "state mutex poisoned"),
    };
    let id = guard.watches.add(name, expr);
    set_result(interp, &format!("w{}", id.0))
}

unsafe extern "C" fn watch_remove_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    if objc != 2 {
        return set_error(
            interp,
            "wrong # args: should be \"::tcld::dbg::watch_remove watch_id\"",
        );
    }
    let id = match unsafe { parse_watch_id_token(objv, 1) } {
        Some(id) => id,
        None => return set_error(interp, "watch_id must be in form w<N>"),
    };
    let Some(state) = step_engine_state(interp) else {
        return set_error(interp, "step engine not installed");
    };
    let mut guard = match state.lock() {
        Ok(g) => g,
        Err(_) => return set_error(interp, "state mutex poisoned"),
    };
    if guard.watches.remove(id).is_none() {
        return set_error(interp, &format!("watch id w{} not found", id.0));
    }
    guard.watch_results.remove(&id);
    set_result(interp, "ok")
}

unsafe extern "C" fn watch_list_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    _objc: c_int,
    _objv: *const *mut Tcl_Obj,
) -> c_int {
    let Some(state) = step_engine_state(interp) else {
        return set_error(interp, "step engine not installed");
    };
    let guard = match state.lock() {
        Ok(g) => g,
        Err(_) => return set_error(interp, "state mutex poisoned"),
    };
    let parts: Vec<String> = guard
        .watches
        .list()
        .map(|w| {
            format!(
                "{{id w{} name {{{}}} expr {{{}}} enabled {}}}",
                w.id.0,
                tcl_escape_braces(&w.name),
                tcl_escape_braces(&w.expr),
                if w.enabled { 1 } else { 0 }
            )
        })
        .collect();
    set_result(interp, &parts.join(" "))
}

unsafe extern "C" fn watch_evaluate_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    // Usage: ::tcld::dbg::watch_evaluate <level>
    // Returns a list of `{id name expr (value <v>)|(error <msg>)}` dicts and
    // updates `state.watch_results`. The host typically calls this
    // from a Paused subscriber, but it works any time the requested
    // frame level is valid.
    if objc != 2 {
        return set_error(
            interp,
            "wrong # args: should be \"::tcld::dbg::watch_evaluate level\"",
        );
    }
    let level = match unsafe { objv_to_int(objv, 1) } {
        Some(n) => n,
        None => return set_error(interp, "level must be an integer"),
    };
    let Some(state) = step_engine_state(interp) else {
        return set_error(interp, "step engine not installed");
    };
    // Collect the snapshot of watches; release the lock before evaluating
    // because eval may itself trigger trace handlers that re-lock.
    let entries: Vec<(tcld_debug_state::WatchId, String, String)> = {
        let guard = match state.lock() {
            Ok(g) => g,
            Err(_) => return set_error(interp, "state mutex poisoned"),
        };
        guard
            .watches
            .list()
            .filter(|w| w.enabled)
            .map(|w| (w.id, w.name.clone(), w.expr.clone()))
            .collect()
    };

    let mut parts = Vec::with_capacity(entries.len());
    let mut results: std::collections::BTreeMap<
        tcld_debug_state::WatchId,
        tcld_debug_state::WatchValue,
    > = Default::default();
    for (id, name, expr) in entries {
        let wrapped = if level <= 0 {
            format!("uplevel #0 [list expr {{{}}}]", expr)
        } else {
            format!("uplevel #{} [list expr {{{}}}]", level, expr)
        };
        let cstr = match CString::new(wrapped) {
            Ok(c) => c,
            Err(_) => {
                let val = tcld_debug_state::WatchValue::Error("expr contains NUL".into());
                results.insert(id, val.clone());
                parts.push(format!(
                    "{{id w{} name {{{}}} expr {{{}}} error {{expr contains NUL}}}}",
                    id.0,
                    tcl_escape_braces(&name),
                    tcl_escape_braces(&expr),
                ));
                continue;
            }
        };
        let rc = unsafe { Tcl_EvalEx(interp, cstr.as_ptr(), -1, 0) };
        let result = unsafe { result_string(interp) };
        if rc == TCL_OK {
            results.insert(id, tcld_debug_state::WatchValue::Ok(result.clone()));
            parts.push(format!(
                "{{id w{} name {{{}}} expr {{{}}} value {{{}}}}}",
                id.0,
                tcl_escape_braces(&name),
                tcl_escape_braces(&expr),
                tcl_escape_braces(&result)
            ));
        } else {
            results.insert(id, tcld_debug_state::WatchValue::Error(result.clone()));
            parts.push(format!(
                "{{id w{} name {{{}}} expr {{{}}} error {{{}}}}}",
                id.0,
                tcl_escape_braces(&name),
                tcl_escape_braces(&expr),
                tcl_escape_braces(&result)
            ));
        }
    }
    if let Ok(mut guard) = state.lock() {
        guard.watch_results = results;
    }
    set_result(interp, &parts.join(" "))
}

unsafe fn parse_watch_id_token(
    objv: *const *mut Tcl_Obj,
    index: usize,
) -> Option<tcld_debug_state::WatchId> {
    let s = unsafe { objv_to_string(objv, index) }?;
    let n = s.strip_prefix("w").and_then(|t| t.parse::<u64>().ok())?;
    Some(tcld_debug_state::WatchId(n))
}

unsafe extern "C" fn bp_export_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    _objc: c_int,
    _objv: *const *mut Tcl_Obj,
) -> c_int {
    let Some(state) = step_engine_state(interp) else {
        return set_error(interp, "step engine not installed");
    };
    let guard = match state.lock() {
        Ok(g) => g,
        Err(_) => return set_error(interp, "state mutex poisoned"),
    };
    let mut parts = Vec::new();
    for bp in guard.breakpoints().list() {
        let block_token = match bp.location.block() {
            BlockId::ProcBody(name) => format!("proc:{name}"),
            BlockId::File(path) => format!("file:{path}"),
            BlockId::Anonymous { content_hash } => format!("anon:{content_hash:016x}"),
        };
        let kind_fields = match &bp.kind {
            BreakpointKind::Line => "kind line".to_string(),
            BreakpointKind::Instruction { pc } => format!("kind instruction pc {pc}"),
            BreakpointKind::Var {
                name,
                on_read,
                on_write,
                on_unset,
            } => {
                let mut ops = String::new();
                if *on_read {
                    ops.push_str("read ");
                }
                if *on_write {
                    ops.push_str("write ");
                }
                if *on_unset {
                    ops.push_str("unset ");
                }
                let ops = ops.trim_end();
                format!(
                    "kind var var_name {{{}}} ops {{{ops}}}",
                    tcl_escape_braces(name)
                )
            }
            BreakpointKind::Exception { uncaught_only } => {
                format!(
                    "kind exception uncaught_only {}",
                    if *uncaught_only { 1 } else { 0 }
                )
            }
        };
        parts.push(format!(
            "{{{kind_fields} block {{{block_token}}} offset {} len {} enabled {} ignore {} temporary {} condition {{{}}}}}",
            bp.location.byte_offset(),
            bp.location.byte_len(),
            if bp.enabled { 1 } else { 0 },
            bp.ignore_count,
            if bp.temporary { 1 } else { 0 },
            tcl_escape_braces(bp.condition.as_deref().unwrap_or("")),
        ));
    }
    set_result(interp, &parts.join(" "))
}

unsafe extern "C" fn bp_import_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    // Usage: ::tcld::dbg::bp_import <serialized>
    // The serialized form is whatever `bp_export` returned: a Tcl list
    // of dicts. Replaces the entire breakpoint store; call
    // `bp_export` first if you want to keep the existing set.
    if objc != 2 {
        return set_error(
            interp,
            "wrong # args: should be \"::tcld::dbg::bp_import serialized\"",
        );
    }
    let serialized = unsafe { objv_to_string(objv, 1) }.unwrap_or_default();
    let entries = parse_bp_dict_list(&serialized);
    let Some(state) = step_engine_state(interp) else {
        return set_error(interp, "step engine not installed");
    };
    let mut guard = match state.lock() {
        Ok(g) => g,
        Err(_) => return set_error(interp, "state mutex poisoned"),
    };
    // Drop existing breakpoints, then re-add. New ids may differ from
    // the exported ones — `bp_export` is a snapshot, not a referential
    // backup.
    let to_remove: Vec<tcld_debug_state::BreakpointId> =
        guard.breakpoints().list().map(|bp| bp.id).collect();
    for id in to_remove {
        guard.with_breakpoints_mut(|store, _| {
            let _ = store.remove(id);
        });
    }
    let mut imported_ids = Vec::with_capacity(entries.len());
    for entry in &entries {
        if !guard.blocks.contains(&entry.block_id) {
            let min_len = entry.offset.saturating_add(entry.len.max(1));
            let placeholder: Arc<str> = Arc::from(" ".repeat(min_len.max(1)));
            let _ = guard
                .blocks
                .register(entry.block_id.clone(), placeholder, vec![entry.offset]);
        }
        let location = match Location::new(
            entry.block_id.clone(),
            0,
            entry.offset..entry.offset + entry.len.max(1),
        ) {
            Ok(l) => l,
            Err(err) => {
                return set_error(
                    interp,
                    &format!("bp_import: {err} (offset {})", entry.offset),
                );
            }
        };
        let kind = entry.kind.clone();
        let condition = if entry.condition.is_empty() {
            None
        } else {
            Some(entry.condition.clone())
        };
        let temporary = entry.temporary;
        let result = guard.with_breakpoints_mut(|store, registry| {
            store.add(kind, location, condition, temporary, registry)
        });
        match result {
            Ok(id) => {
                if entry.ignore_count > 0 {
                    guard.with_breakpoints_mut(|store, _| {
                        let _ = store.set_ignore_count(id, entry.ignore_count);
                    });
                }
                if !entry.enabled {
                    guard.with_breakpoints_mut(|store, _| {
                        let _ = store.disable(id);
                    });
                }
                imported_ids.push(id);
            }
            Err(err) => {
                return set_error(interp, &format!("bp_import: {err}"));
            }
        }
    }
    let formatted: Vec<String> = imported_ids
        .iter()
        .map(|id| format!("bp{}", id.0))
        .collect();
    drop(guard);
    if let Err(err) = resync_opcode_arm(interp) {
        return set_error(interp, &err);
    }
    set_result(interp, &formatted.join(" "))
}

#[derive(Debug, Clone)]
struct BpImportEntry {
    kind: BreakpointKind,
    block_id: BlockId,
    offset: usize,
    len: usize,
    enabled: bool,
    ignore_count: u64,
    temporary: bool,
    condition: String,
}

fn parse_bp_dict_list(text: &str) -> Vec<BpImportEntry> {
    // Each top-level token is a dict; tokenize using simple_dict_tokens
    // (already brace-aware). Each dict is then itself a flat key/value
    // list parseable the same way.
    let dicts = simple_dict_tokens(text);
    let mut out = Vec::with_capacity(dicts.len());
    for dict in dicts {
        let kvs = simple_dict_tokens(&dict);
        let mut kind_str = String::new();
        let mut block_token = String::new();
        let mut offset: usize = 0;
        let mut len: usize = 1;
        let mut enabled = true;
        let mut ignore_count: u64 = 0;
        let mut temporary = false;
        let mut condition = String::new();
        let mut var_name = String::new();
        let mut ops = String::new();
        let mut uncaught_only = false;
        let mut pc: usize = 0;
        let mut iter = kvs.into_iter();
        while let (Some(k), Some(v)) = (iter.next(), iter.next()) {
            match k.as_str() {
                "kind" => kind_str = v,
                "block" => block_token = v,
                "offset" => offset = v.parse().unwrap_or(0),
                "len" => len = v.parse().unwrap_or(1),
                "enabled" => enabled = v != "0",
                "ignore" => ignore_count = v.parse().unwrap_or(0),
                "temporary" => temporary = v != "0",
                "condition" => condition = v,
                "var_name" => var_name = v,
                "ops" => ops = v,
                "uncaught_only" => uncaught_only = v != "0",
                "pc" => pc = v.parse().unwrap_or(0),
                _ => {}
            }
        }
        let block_id = if let Some(rest) = block_token.strip_prefix("proc:") {
            BlockId::ProcBody(Arc::from(rest))
        } else if let Some(rest) = block_token.strip_prefix("file:") {
            BlockId::File(Arc::from(rest))
        } else if let Some(rest) = block_token.strip_prefix("anon:") {
            let hash = u64::from_str_radix(rest, 16).unwrap_or(0);
            BlockId::Anonymous { content_hash: hash }
        } else {
            // Fall back to ProcBody for unknown tokens.
            BlockId::ProcBody(Arc::from(block_token))
        };
        let kind = match kind_str.as_str() {
            "var" => {
                let on_read = ops.split_whitespace().any(|t| t == "read");
                let on_write = ops.split_whitespace().any(|t| t == "write");
                let on_unset = ops.split_whitespace().any(|t| t == "unset");
                BreakpointKind::Var {
                    name: var_name,
                    on_read,
                    on_write,
                    on_unset,
                }
            }
            "exception" => BreakpointKind::Exception { uncaught_only },
            "instruction" => BreakpointKind::Instruction { pc },
            _ => BreakpointKind::Line,
        };
        out.push(BpImportEntry {
            kind,
            block_id,
            offset,
            len,
            enabled,
            ignore_count,
            temporary,
            condition,
        });
    }
    out
}

unsafe extern "C" fn slave_list_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    _objc: c_int,
    _objv: *const *mut Tcl_Obj,
) -> c_int {
    let Some(state) = step_engine_state(interp) else {
        return set_error(interp, "step engine not installed");
    };
    let guard = match state.lock() {
        Ok(g) => g,
        Err(_) => return set_error(interp, "state mutex poisoned"),
    };
    let parts: Vec<String> = guard
        .slaves
        .iter()
        .map(|n| format!("{{{}}}", tcl_escape_braces(n)))
        .collect();
    set_result(interp, &parts.join(" "))
}

unsafe extern "C" fn slave_install_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    // Usage: ::tcld::dbg::slave_install <slave_name>
    // Resolves the slave interp via `Tcl_GetSlave`, installs the engine
    // on it sharing this interp's DebugState (so breakpoints / events
    // are unified), and registers the public Tcl commands on the slave.
    if objc != 2 {
        return set_error(
            interp,
            "wrong # args: should be \"::tcld::dbg::slave_install slave_name\"",
        );
    }
    let name = match unsafe { objv_to_string(objv, 1) } {
        Some(s) => s,
        None => return set_error(interp, "slave_name missing"),
    };
    let cname = match make_cstring(&name) {
        Ok(c) => c,
        Err(err) => return set_error(interp, &err),
    };
    let slave_interp = unsafe { tcl_sys::Tcl_GetChild(interp, cname.as_ptr()) };
    if slave_interp.is_null() {
        return set_error(interp, &format!("no such slave interpreter: {name}"));
    }
    let Some(state) = step_engine_state(interp) else {
        return set_error(interp, "step engine not installed");
    };
    if let Err(err) = unsafe { step_engine::install(slave_interp, state) } {
        return set_error(interp, &err);
    }
    if let Err(err) = unsafe { install_tier2_helpers(slave_interp) } {
        return set_error(interp, &err);
    }
    // Register the public commands on the slave so the same Tcl-level
    // surface is available there too. If the slave has any of these
    // names already (because it was created with -safe), createObjCommand
    // overrides them; that's the same behavior as the parent.
    if unsafe { Tcld_dbg_Init(slave_interp) } != TCL_OK {
        return set_error(interp, "failed to init slave engine commands");
    }
    // Install command wrappers on the slave too — otherwise procs
    // defined inside `interp eval child {proc x ...}` go through the
    // child's unwrapped `proc` command and never register a Block in
    // the shared state.
    if let Err(err) = unsafe { wrappers::install_command_wrappers(slave_interp) } {
        return set_error(interp, &err);
    }
    set_result(interp, "ok")
}

// ─── TclPro parity completion: minor commands ─────────────────────────
//
// Round out the in-process surface to match TclPro's `dbg::*` API:
// - get_level (current Tcl frame depth)
// - quit (uninstall engine)
// - register / unregister (Tcl-side event subscription)
// - var_breakpoints / line_breakpoints (filtered list)
// - pre_instrument_proc / pre_instrument_file (explicit Block registration
//   for code that hasn't been sourced yet)

unsafe extern "C" fn get_level_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    _objc: c_int,
    _objv: *const *mut Tcl_Obj,
) -> c_int {
    // Mirror TclPro's `dbg::getLevel`. Returns the current Tcl frame
    // depth (`info level`). Gates on Stopped to match TclPro's
    // contract — getLevel is meaningful only when the interpreter has
    // paused.
    if let Some(state) = step_engine_state(interp) {
        let guard = match state.lock() {
            Ok(g) => g,
            Err(_) => return set_error(interp, "state mutex poisoned"),
        };
        if let Err(err) = guard.require_stopped() {
            return set_error(interp, &err.to_string());
        }
    }
    let script = b"info level\0";
    let rc = unsafe {
        Tcl_EvalEx(
            interp,
            script.as_ptr() as *const std::os::raw::c_char,
            -1,
            0,
        )
    };
    if rc != TCL_OK {
        return set_error(interp, "info level failed");
    }
    TCL_OK
}

unsafe extern "C" fn quit_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    _objc: c_int,
    _objv: *const *mut Tcl_Obj,
) -> c_int {
    // Mirror TclPro's `dbg::quit`. Uninstalls the engine: drops the
    // trace, clears app_state to Dead, releases the EngineCtx. Idempotent
    // — calling on a non-attached interp is a no-op (returns ok).
    let _ = unsafe { step_engine::uninstall(interp) };
    set_result(interp, "ok")
}

unsafe extern "C" fn register_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    // Usage: ::tcld::dbg::register <event_filter> <script>
    // event_filter: paused | resumed | errorpaused | conditionerror |
    //               blocksrevalidated | all
    // Returns: sub<N> id usable with `unregister`.
    if objc != 3 {
        return set_error(
            interp,
            "wrong # args: should be \"::tcld::dbg::register event script\"",
        );
    }
    let filter = match unsafe { objv_to_string(objv, 1) } {
        Some(s) => s,
        None => return set_error(interp, "event filter missing"),
    };
    let script = match unsafe { objv_to_string(objv, 2) } {
        Some(s) => s,
        None => return set_error(interp, "script missing"),
    };
    let Some(state) = step_engine_state(interp) else {
        return set_error(interp, "step engine not installed");
    };
    let interp_addr = interp as usize;
    let cb: tcld_debug_state::EventCallback = Arc::new(move |event| {
        if !event_matches_filter(&filter, &event) {
            return;
        }
        // Safety: the trace handler / wrapper paths that emit events
        // run on the Tcl thread that owns `interp`, so we may
        // synchronously Tcl_EvalEx here. External-thread emitters
        // would need a different bridge; not in scope today.
        let interp = interp_addr as *mut Tcl_Interp;
        if let Ok(cstr) = CString::new(script.as_str()) {
            unsafe {
                Tcl_EvalEx(interp, cstr.as_ptr(), -1, 0);
            }
        }
    });
    let id = match state.lock() {
        Ok(mut guard) => guard.subscribe(cb),
        Err(_) => return set_error(interp, "state mutex poisoned"),
    };
    set_result(interp, &format!("sub{}", id.0))
}

fn event_matches_filter(filter: &str, event: &tcld_debug_state::DebugEvent) -> bool {
    use tcld_debug_state::DebugEvent;
    match filter {
        "all" | "*" | "" => true,
        "paused" => matches!(event, DebugEvent::Paused { .. }),
        "resumed" => matches!(event, DebugEvent::Resumed),
        "errorpaused" => matches!(event, DebugEvent::ErrorPaused { .. }),
        "conditionerror" => matches!(event, DebugEvent::ConditionError { .. }),
        "blocksrevalidated" => matches!(event, DebugEvent::BlocksRevalidated { .. }),
        _ => false,
    }
}

unsafe extern "C" fn unregister_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    if objc != 2 {
        return set_error(
            interp,
            "wrong # args: should be \"::tcld::dbg::unregister sub_id\"",
        );
    }
    let sub_token = match unsafe { objv_to_string(objv, 1) } {
        Some(s) => s,
        None => return set_error(interp, "sub_id missing"),
    };
    let id = match sub_token
        .strip_prefix("sub")
        .and_then(|t| t.parse::<u64>().ok())
    {
        Some(n) => tcld_debug_state::SubscriptionId(n),
        None => return set_error(interp, "sub_id must be in form sub<N>"),
    };
    let Some(state) = step_engine_state(interp) else {
        return set_error(interp, "step engine not installed");
    };
    let removed = match state.lock() {
        Ok(mut g) => g.unsubscribe(id),
        Err(_) => return set_error(interp, "state mutex poisoned"),
    };
    if removed {
        set_result(interp, "ok")
    } else {
        set_error(interp, &format!("subscription not found: {sub_token}"))
    }
}

unsafe extern "C" fn var_breakpoints_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    // Usage: ::tcld::dbg::var_breakpoints ?<name>?
    // Lists Var-kind breakpoints, optionally filtered by var name.
    if objc > 2 {
        return set_error(
            interp,
            "wrong # args: should be \"::tcld::dbg::var_breakpoints ?name?\"",
        );
    }
    let name_filter = if objc == 2 {
        unsafe { objv_to_string(objv, 1) }
    } else {
        None
    };
    let Some(state) = step_engine_state(interp) else {
        return set_error(interp, "step engine not installed");
    };
    let guard = match state.lock() {
        Ok(g) => g,
        Err(_) => return set_error(interp, "state mutex poisoned"),
    };
    let mut parts = Vec::new();
    for bp in guard.breakpoints().list() {
        let BreakpointKind::Var {
            name,
            on_read,
            on_write,
            on_unset,
        } = &bp.kind
        else {
            continue;
        };
        if let Some(n) = &name_filter
            && n != name
        {
            continue;
        }
        let mut ops = String::new();
        if *on_read {
            ops.push_str("read ");
        }
        if *on_write {
            ops.push_str("write ");
        }
        if *on_unset {
            ops.push_str("unset ");
        }
        parts.push(format!(
            "{{id bp{} name {{{}}} ops {{{}}} enabled {} hits {} ignore {}}}",
            bp.id.0,
            tcl_escape_braces(name),
            ops.trim_end(),
            if bp.enabled { 1 } else { 0 },
            bp.hit_count,
            bp.ignore_count,
        ));
    }
    set_result(interp, &parts.join(" "))
}

unsafe extern "C" fn line_breakpoints_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    // Usage: ::tcld::dbg::line_breakpoints ?<qname>? ?<offset>?
    // Lists Line-kind breakpoints, optionally filtered by proc qname and/or offset.
    if objc > 3 {
        return set_error(
            interp,
            "wrong # args: should be \"::tcld::dbg::line_breakpoints ?qname? ?offset?\"",
        );
    }
    let qname_filter = if objc >= 2 {
        unsafe { objv_to_string(objv, 1) }
    } else {
        None
    };
    let offset_filter = if objc == 3 {
        match unsafe { objv_to_int(objv, 2) } {
            Some(n) if n >= 0 => Some(n as usize),
            _ => return set_error(interp, "offset must be non-negative integer"),
        }
    } else {
        None
    };
    let Some(state) = step_engine_state(interp) else {
        return set_error(interp, "step engine not installed");
    };
    let guard = match state.lock() {
        Ok(g) => g,
        Err(_) => return set_error(interp, "state mutex poisoned"),
    };
    let bare_filter = qname_filter
        .as_deref()
        .map(|q| q.trim_start_matches("::").to_string());
    let mut parts = Vec::new();
    for bp in guard.breakpoints().list() {
        if !matches!(bp.kind, BreakpointKind::Line) {
            continue;
        }
        let block_token = match bp.location.block() {
            BlockId::ProcBody(name) => {
                if let Some(filter) = &bare_filter
                    && filter.as_str() != name.as_ref()
                {
                    continue;
                }
                format!("proc:{name}")
            }
            BlockId::File(path) => {
                if let Some(filter) = &bare_filter
                    && filter.as_str() != path.as_ref()
                {
                    continue;
                }
                format!("file:{path}")
            }
            BlockId::Anonymous { content_hash } => {
                if bare_filter.is_some() {
                    continue;
                }
                format!("anon:{content_hash:016x}")
            }
        };
        let off = bp.location.byte_offset();
        if let Some(want) = offset_filter
            && want != off
        {
            continue;
        }
        let id_part = bp.id.0;
        let len_part = bp.location.byte_len();
        let enabled_part = if bp.enabled { 1 } else { 0 };
        let hits_part = bp.hit_count;
        let ignore_part = bp.ignore_count;
        let cond_part = match bp.condition.as_deref() {
            Some(c) if !c.is_empty() => format!(" condition {{{}}}", tcl_escape_braces(c)),
            _ => String::new(),
        };
        parts.push(format!(
            "{{id bp{id_part} block {{{block_token}}} offset {off} len {len_part} enabled {enabled_part} hits {hits_part} ignore {ignore_part}{cond_part}}}",
        ));
    }
    set_result(interp, &parts.join(" "))
}

unsafe extern "C" fn instruction_breakpoints_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    if !(1..=2).contains(&objc) {
        return set_error(
            interp,
            "wrong # args: should be \"::tcld::dbg::instruction_breakpoints ?pc?\"",
        );
    }
    let pc_filter = if objc == 2 {
        match unsafe { objv_to_int(objv, 1) } {
            Some(n) if n >= 0 => Some(n as usize),
            _ => return set_error(interp, "pc must be a non-negative integer"),
        }
    } else {
        None
    };
    let Some(state) = step_engine_state(interp) else {
        return set_error(interp, "step engine not installed");
    };
    let guard = match state.lock() {
        Ok(g) => g,
        Err(_) => return set_error(interp, "state mutex poisoned"),
    };
    let mut parts = Vec::new();
    for bp in guard.breakpoints().list() {
        let BreakpointKind::Instruction { pc } = bp.kind else {
            continue;
        };
        if let Some(filter) = pc_filter
            && filter != pc
        {
            continue;
        }
        parts.push(format!(
            "{{id bp{} kind instruction pc {} enabled {} hits {} ignore {}}}",
            bp.id.0,
            pc,
            if bp.enabled { 1 } else { 0 },
            bp.hit_count,
            bp.ignore_count,
        ));
    }
    set_result(interp, &parts.join(" "))
}

unsafe extern "C" fn pre_instrument_proc_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    // Usage: ::tcld::dbg::pre_instrument_proc <qname> <body>
    // Pre-registers a proc body Block so the host can set breakpoints
    // before the proc is actually defined. Idempotent for identical
    // bodies; replaces (and bumps version) for different bodies.
    if objc != 3 {
        return set_error(
            interp,
            "wrong # args: should be \"::tcld::dbg::pre_instrument_proc qname body\"",
        );
    }
    let qname = match unsafe { objv_to_string(objv, 1) } {
        Some(s) => s,
        None => return set_error(interp, "qname missing"),
    };
    let body = unsafe { objv_to_string(objv, 2) }.unwrap_or_default();
    let bare = qname.trim_start_matches("::");
    let Some(state) = step_engine_state(interp) else {
        return set_error(interp, "step engine not installed");
    };
    let mut guard = match state.lock() {
        Ok(g) => g,
        Err(_) => return set_error(interp, "state mutex poisoned"),
    };
    let registered_id =
        match tcld_debug_state::register_body_block(&mut guard.blocks, Some(bare), &body) {
            Ok(id) => id,
            Err(err) => return set_error(interp, &err.to_string()),
        };
    // Re-validate any breakpoints already attached to this block.
    let block_clone = guard.blocks.get(&registered_id).cloned();
    if let Some(block) = block_clone {
        let _ = guard.with_breakpoints_mut(|store, _| store.validate_against(&block));
    }
    let token = match &registered_id {
        BlockId::ProcBody(n) => format!("proc:{n}"),
        BlockId::File(p) => format!("file:{p}"),
        BlockId::Anonymous { content_hash } => format!("anon:{content_hash:016x}"),
    };
    set_result(interp, &token)
}

unsafe extern "C" fn pre_instrument_file_cmd(
    _cd: *mut c_void,
    interp: *mut Tcl_Interp,
    objc: c_int,
    objv: *const *mut Tcl_Obj,
) -> c_int {
    // Usage: ::tcld::dbg::pre_instrument_file <path> <body>
    // Pre-registers a File-kind Block. Mirrors what the `source`
    // wrapper does on actual sourcing — useful when the host has the
    // file contents up-front (e.g., loaded via a lazy loader).
    if objc != 3 {
        return set_error(
            interp,
            "wrong # args: should be \"::tcld::dbg::pre_instrument_file path body\"",
        );
    }
    let path = match unsafe { objv_to_string(objv, 1) } {
        Some(s) => s,
        None => return set_error(interp, "path missing"),
    };
    let body = unsafe { objv_to_string(objv, 2) }.unwrap_or_default();
    let Some(state) = step_engine_state(interp) else {
        return set_error(interp, "step engine not installed");
    };
    let mut guard = match state.lock() {
        Ok(g) => g,
        Err(_) => return set_error(interp, "state mutex poisoned"),
    };
    // Parse the body for command spans, same as the `source` wrapper.
    let parse_input = if body.ends_with('\n') {
        body.clone()
    } else {
        format!("{body}\n")
    };
    let spans = tcld_syntax::command_spans(&parse_input).unwrap_or_default();
    let mut offsets: Vec<usize> = spans.iter().map(|s| s.start_byte).collect();
    offsets.sort_unstable();
    offsets.dedup();
    let id = BlockId::File(Arc::from(path.as_str()));
    if let Err(err) =
        guard
            .blocks
            .register_replacement(id.clone(), Arc::from(body.as_str()), offsets)
    {
        return set_error(interp, &err.to_string());
    }
    let block = guard.blocks.get(&id).cloned();
    if let Some(block) = block {
        let _ = guard.with_breakpoints_mut(|store, _| store.validate_against(&block));
    }
    set_result(interp, &format!("file:{path}"))
}

/// # Safety
///
/// `interp` must be a valid, non-null Tcl interpreter pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Tcld_dbg_Init(interp: *mut Tcl_Interp) -> c_int {
    if interp.is_null() {
        return TCL_ERROR;
    }

    for (name, proc_fn) in [
        (
            "::tcld::dbg::capabilities",
            capabilities_cmd
                as unsafe extern "C" fn(
                    *mut c_void,
                    *mut Tcl_Interp,
                    c_int,
                    *const *mut Tcl_Obj,
                ) -> c_int,
        ),
        ("::tcld::dbg::install", install_cmd),
        ("::tcld::dbg::step_instruction", step_instruction_cmd),
        (
            "::tcld::dbg::step_over_instruction",
            step_over_instruction_cmd,
        ),
        (
            "::tcld::dbg::step_out_instruction",
            step_out_instruction_cmd,
        ),
        ("::tcld::dbg::run_to_pc", run_to_pc_cmd),
        ("::tcld::dbg::profile_start", profile_start_cmd),
        ("::tcld::dbg::profile_stop", profile_stop_cmd),
        ("::tcld::dbg::profile_reset", profile_reset_cmd),
        ("::tcld::dbg::profile_data", profile_data_cmd),
        ("::tcld::dbg::current_pc", current_pc_cmd),
        ("::tcld::dbg::version", version_cmd),
        ("::tcld::dbg::has", has_capability_cmd),
        ("::tcld::dbg::engine_install", engine_install_cmd),
        ("::tcld::dbg::engine_uninstall", engine_uninstall_cmd),
        ("::tcld::dbg::step_into", step_into_cmd),
        ("::tcld::dbg::step_over", step_over_cmd),
        ("::tcld::dbg::step_out", step_out_cmd),
        ("::tcld::dbg::continue", continue_cmd),
        ("::tcld::dbg::breakpoint_add", breakpoint_add_cmd),
        ("::tcld::dbg::file_line_location", file_line_location_cmd),
        ("::tcld::dbg::breakpoint_list", breakpoint_list_cmd),
        ("::tcld::dbg::app_state", app_state_cmd),
        (
            "::tcld::dbg::install_command_wrappers",
            install_command_wrappers_cmd,
        ),
        ("::tcld::dbg::inspect_stack", inspect_stack_cmd),
        ("::tcld::dbg::eval_in_frame", eval_in_frame_cmd),
        ("::tcld::dbg::watchpoint_add", watchpoint_add_cmd),
        ("::tcld::dbg::coverage", coverage_cmd),
        ("::tcld::dbg::coverage_enable", coverage_enable_cmd),
        // ── Tier 2 ──
        ("::tcld::dbg::get_variables", get_variables_cmd),
        ("::tcld::dbg::get_var", get_var_cmd),
        ("::tcld::dbg::set_var", set_var_cmd),
        ("::tcld::dbg::get_result", get_result_cmd),
        ("::tcld::dbg::list_procs", list_procs_cmd),
        ("::tcld::dbg::proc_body", proc_body_cmd),
        ("::tcld::dbg::block_source", block_source_cmd),
        ("::tcld::dbg::block_generation", block_generation_cmd),
        ("::tcld::dbg::block_command_span", block_command_span_cmd),
        (
            "::tcld::dbg::__bind_logical_breakpoint",
            bind_logical_breakpoint_cmd,
        ),
        (
            "::tcld::dbg::__unbind_logical_breakpoint",
            unbind_logical_breakpoint_cmd,
        ),
        ("::tcld::dbg::proc_location", proc_location_cmd),
        ("::tcld::dbg::block_disable", block_disable_cmd),
        ("::tcld::dbg::block_enable", block_enable_cmd),
        ("::tcld::dbg::ignore_error", ignore_error_cmd),
        ("::tcld::dbg::bp_remove", bp_remove_cmd),
        ("::tcld::dbg::bp_disable", bp_disable_cmd),
        ("::tcld::dbg::bp_enable", bp_enable_cmd),
        ("::tcld::dbg::bp_set_ignore", bp_set_ignore_cmd),
        ("::tcld::dbg::bp_move", bp_move_cmd),
        ("::tcld::dbg::bp_validate", bp_validate_cmd),
        // ── Tier 3 ──
        ("::tcld::dbg::request_pause", request_pause_cmd),
        (
            "::tcld::dbg::activate_target_file",
            activate_target_file_cmd,
        ),
        (
            "::tcld::dbg::clear_active_target_file",
            clear_active_target_file_cmd,
        ),
        (
            "::tcld::dbg::pause_at_target_entry",
            pause_at_target_entry_cmd,
        ),
        ("::tcld::dbg::run_to", run_to_cmd),
        ("::tcld::dbg::watch_add", watch_add_cmd),
        ("::tcld::dbg::watch_remove", watch_remove_cmd),
        ("::tcld::dbg::watch_list", watch_list_cmd),
        ("::tcld::dbg::watch_evaluate", watch_evaluate_cmd),
        ("::tcld::dbg::bp_export", bp_export_cmd),
        ("::tcld::dbg::bp_import", bp_import_cmd),
        ("::tcld::dbg::slave_list", slave_list_cmd),
        ("::tcld::dbg::slave_install", slave_install_cmd),
        // ── TclPro parity completion ──
        ("::tcld::dbg::get_level", get_level_cmd),
        ("::tcld::dbg::quit", quit_cmd),
        ("::tcld::dbg::register", register_cmd),
        ("::tcld::dbg::unregister", unregister_cmd),
        ("::tcld::dbg::var_breakpoints", var_breakpoints_cmd),
        ("::tcld::dbg::line_breakpoints", line_breakpoints_cmd),
        (
            "::tcld::dbg::instruction_breakpoints",
            instruction_breakpoints_cmd,
        ),
        ("::tcld::dbg::pre_instrument_proc", pre_instrument_proc_cmd),
        ("::tcld::dbg::pre_instrument_file", pre_instrument_file_cmd),
        // ── Tier 4: Nub-side wire bridge ──
        ("::tcld::dbg::nub_connect", nub::nub_connect_cmd),
        ("::tcld::dbg::nub_status", nub::nub_status_cmd),
        ("::tcld::dbg::nub_disconnect", nub::nub_disconnect_cmd),
    ] {
        if let Err(err) = create_command(interp, name, proc_fn) {
            return set_error(interp, &err);
        }
    }

    let Ok(pkg_name) = make_cstring("tcld_dbg") else {
        return TCL_ERROR;
    };
    let Ok(pkg_version) = make_cstring(EXT_VERSION) else {
        return TCL_ERROR;
    };

    unsafe {
        if Tcl_PkgProvide(interp, pkg_name.as_ptr(), pkg_version.as_ptr()) != TCL_OK {
            return TCL_ERROR;
        }
    }

    TCL_OK
}

/// # Safety
///
/// `interp` must be a valid, non-null Tcl interpreter pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Tcld_dbg_SafeInit(interp: *mut Tcl_Interp) -> c_int {
    unsafe { Tcld_dbg_Init(interp) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_family_parsing() {
        assert_eq!(parse_runtime_family("8.5.19"), RuntimeFamily::Tcl85);
        assert_eq!(parse_runtime_family("8.6.14"), RuntimeFamily::Tcl86);
        assert_eq!(parse_runtime_family("9.0.0"), RuntimeFamily::Tcl90);
        assert_eq!(parse_runtime_family("9.1.0"), RuntimeFamily::Tcl90);
        assert_eq!(parse_runtime_family("invalid"), RuntimeFamily::Unknown);
    }

    #[test]
    fn top_level_file_instrumentation_registers_each_command() {
        let source = "exec touch $::env(TCLD_DEBUG_PTY_ENTRY_SENTINEL)\nexec touch $::env(TCLD_DEBUG_PTY_BREAKPOINT_SENTINEL)\nset completed 1\n";
        let spans = tcld_syntax::command_spans(source).expect("parse top-level Tcl source");
        let commands = spans
            .iter()
            .map(|span| source[span.start_byte..].lines().next().unwrap_or_default())
            .collect::<Vec<_>>();
        assert_eq!(
            commands,
            [
                "exec touch $::env(TCLD_DEBUG_PTY_ENTRY_SENTINEL)",
                "exec touch $::env(TCLD_DEBUG_PTY_BREAKPOINT_SENTINEL)",
                "set completed 1",
            ]
        );
    }

    #[test]
    fn adapter_selection_by_patchlevel() {
        let sel85 = select_adapter_for_patchlevel("8.5.19".to_string());
        assert_eq!(sel85.adapter.id(), "adapter_tcl85");

        let sel86 = select_adapter_for_patchlevel("8.6.14".to_string());
        assert_eq!(sel86.adapter.id(), "adapter_tcl86");

        let sel90 = select_adapter_for_patchlevel("9.0.3".to_string());
        assert_eq!(sel90.adapter.id(), "adapter_tcl90");

        let sel_unknown = select_adapter_for_patchlevel("8.7a5".to_string());
        assert_eq!(sel_unknown.adapter.id(), "adapter_unknown");
    }

    #[test]
    fn resolved_capabilities_include_runtime_flags() {
        let selected = select_adapter_for_patchlevel("8.6.14".to_string());
        let caps = resolved_capabilities(std::ptr::null_mut(), &selected);
        assert_eq!(caps.get("runtime_tcl8_6"), Some(true));
        assert_eq!(caps.get("runtime_tcl8_5"), Some(false));
        assert_eq!(caps.get("runtime_tcl9"), Some(false));
        assert_eq!(caps.get("command_step"), Some(true));
    }

    #[test]
    fn resolved_capabilities_include_tcl9_flags() {
        let selected = select_adapter_for_patchlevel("9.0.3".to_string());
        let caps = resolved_capabilities(std::ptr::null_mut(), &selected);
        assert_eq!(caps.get("runtime_tcl9"), Some(true));
        assert_eq!(caps.get("adapter_tcl90"), Some(true));
        assert_eq!(caps.get("runtime_supported"), Some(true));
        assert_eq!(caps.get("instruction_step"), Some(false));
    }

    #[test]
    fn tcl90_capabilities_include_obj_introspection() {
        let selected = select_adapter_for_patchlevel("9.0.3".to_string());
        let caps = resolved_capabilities(std::ptr::null_mut(), &selected);
        assert_eq!(caps.get("obj_introspection"), Some(true));
    }

    #[test]
    fn tcl86_capabilities_exclude_obj_introspection() {
        let selected = select_adapter_for_patchlevel("8.6.14".to_string());
        let caps = resolved_capabilities(std::ptr::null_mut(), &selected);
        assert_eq!(caps.get("obj_introspection"), None);
    }

    #[test]
    fn parse_representation_pure_string() {
        let repr = r#"value is a pure string with a refcount of 2, object pointer at 0x1234, no internal representation, string representation "hello""#;
        let info = parse_representation(repr, true);
        assert_eq!(info.type_name, "none");
        assert!(!info.has_internal_rep);
        assert!(info.has_string_rep);
        assert_eq!(info.ref_count, 2);
    }

    #[test]
    fn parse_representation_int_type() {
        let repr = r#"value is a int with a refcount of 3, object pointer at 0x5678, internal representation 0x000000000000002a, string representation "42""#;
        let info = parse_representation(repr, true);
        assert_eq!(info.type_name, "int");
        assert!(info.has_internal_rep);
        assert!(info.has_string_rep);
        assert_eq!(info.ref_count, 3);
    }

    #[test]
    fn parse_representation_list_type() {
        let repr = r#"value is a list with a refcount of 1, object pointer at 0xabcd, internal representation 0x1234, string representation "a b c""#;
        let info = parse_representation(repr, true);
        assert_eq!(info.type_name, "list");
        assert!(info.has_internal_rep);
        assert_eq!(info.ref_count, 1);
    }

    #[test]
    fn parse_representation_no_string_rep() {
        let repr = r#"value is a dict with a refcount of 1, object pointer at 0xabcd, internal representation 0x1234, no string representation"#;
        let info = parse_representation(repr, false);
        assert_eq!(info.type_name, "dict");
        assert!(info.has_internal_rep);
        assert!(!info.has_string_rep);
    }

    #[test]
    fn format_intrep_dict_roundtrip() {
        let info = ObjIntrep {
            type_name: "list".into(),
            has_string_rep: true,
            has_internal_rep: true,
            ref_count: 3,
            detail: "elements=5".into(),
        };
        let dict = format_intrep_dict(&info);
        assert!(dict.contains("type {list}"));
        assert!(dict.contains("hasStringRep 1"));
        assert!(dict.contains("hasInternalRep 1"));
        assert!(dict.contains("refCount 3"));
        assert!(dict.contains("detail {elements=5}"));
    }

    #[test]
    fn split_tcl_list_simple_basic() {
        let items = split_tcl_list_simple("a b {c d} e");
        assert_eq!(items, vec!["a", "b", "c d", "e"]);
    }

    #[test]
    fn split_tcl_list_simple_empty() {
        let items = split_tcl_list_simple("");
        assert!(items.is_empty());
    }

    #[test]
    fn tcl_escape_braces_special_chars() {
        assert_eq!(tcl_escape_braces("hello"), "hello");
        assert_eq!(tcl_escape_braces("a{b}c"), "a\\{b\\}c");
        assert_eq!(tcl_escape_braces("a\\b"), "a\\\\b");
    }

    #[test]
    fn tcl90_capabilities_include_command_resolution() {
        let selected = select_adapter_for_patchlevel("9.0.3".to_string());
        let caps = resolved_capabilities(std::ptr::null_mut(), &selected);
        assert_eq!(caps.get("command_resolution"), Some(true));
    }

    #[test]
    fn tcl86_capabilities_exclude_command_resolution() {
        let selected = select_adapter_for_patchlevel("8.6.14".to_string());
        let caps = resolved_capabilities(std::ptr::null_mut(), &selected);
        assert_eq!(caps.get("command_resolution"), Some(false));
    }

    #[test]
    fn tcl85_capabilities_exclude_command_resolution() {
        let selected = select_adapter_for_patchlevel("8.5.19".to_string());
        let caps = resolved_capabilities(std::ptr::null_mut(), &selected);
        assert_eq!(caps.get("command_resolution"), Some(false));
    }

    #[test]
    fn effective_command_type_prefers_alias() {
        let ty = effective_command_type("native", true, true, true);
        assert_eq!(ty, "alias");
    }

    #[test]
    fn effective_command_type_prefers_ensemble_over_import() {
        let ty = effective_command_type("native", true, true, false);
        assert_eq!(ty, "ensemble");
    }

    #[test]
    fn effective_command_type_marks_import() {
        let ty = effective_command_type("proc", true, false, false);
        assert_eq!(ty, "import");
    }

    #[test]
    fn extract_namespace_from_fqn() {
        assert_eq!(extract_namespace("::foo"), "::");
        assert_eq!(extract_namespace("::mylib::connect"), "::mylib");
        assert_eq!(extract_namespace("::a::b::c"), "::a::b");
        assert_eq!(extract_namespace("plain"), "::");
    }

    #[test]
    fn format_resolution_dict_not_found() {
        let dict = format_resolution_dict(
            "nonexistent",
            "",
            "::",
            "",
            false,
            None,
            &[],
            false,
            None,
            false,
            None,
            "unknown",
            false,
            &[],
        );
        assert!(dict.contains("originalName {nonexistent}"));
        assert!(dict.contains("exists 0"));
        assert!(dict.contains("commandType {unknown}"));
        assert!(dict.contains("isImported 0"));
        assert!(dict.contains("isEnsemble 0"));
        assert!(dict.contains("isAlias 0"));
    }

    #[test]
    fn format_resolution_dict_found_proc() {
        let chain = vec!["::foo".to_string()];
        let path = vec!["{step lookup namespace {::} name {foo} result found}".to_string()];
        let dict = format_resolution_dict(
            "foo", "::foo", "::", "::", false, None, &chain, false, None, false, None, "proc",
            true, &path,
        );
        assert!(dict.contains("originalName {foo}"));
        assert!(dict.contains("resolvedName {::foo}"));
        assert!(dict.contains("exists 1"));
        assert!(dict.contains("commandType {proc}"));
        assert!(dict.contains("isImported 0"));
    }

    #[test]
    fn format_resolution_dict_imported() {
        let chain = vec![
            "::app::handler".to_string(),
            "::internal::handler".to_string(),
        ];
        let dict = format_resolution_dict(
            "handler",
            "::internal::handler",
            "::app",
            "::internal",
            true,
            Some("::internal::handler"),
            &chain,
            false,
            None,
            false,
            None,
            "proc",
            true,
            &[],
        );
        assert!(dict.contains("isImported 1"));
        assert!(dict.contains("importSource {::internal::handler}"));
        assert!(dict.contains("importChain {{::app::handler} {::internal::handler}}"));
    }

    #[test]
    fn format_resolution_dict_ensemble() {
        let dict = format_resolution_dict(
            "string",
            "::string",
            "::",
            "::",
            false,
            None,
            &["::string".to_string()],
            true,
            Some("::tcl::string"),
            false,
            None,
            "native",
            true,
            &[],
        );
        assert!(dict.contains("isEnsemble 1"));
        assert!(dict.contains("ensembleNamespace {::tcl::string}"));
        assert!(dict.contains("commandType {native}"));
    }

    #[test]
    fn format_resolution_dict_alias() {
        let dict = format_resolution_dict(
            "mylog",
            "::mylog",
            "::",
            "::",
            false,
            None,
            &["::mylog".to_string()],
            false,
            None,
            true,
            Some("::logging::info"),
            "native",
            true,
            &[],
        );
        assert!(dict.contains("isAlias 1"));
        assert!(dict.contains("aliasTarget {::logging::info}"));
    }
}
