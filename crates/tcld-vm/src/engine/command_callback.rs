//! Synchronous command callbacks with a Tcl scope and a mapped VM continuation.
use super::*;

impl HybridEngineInner {
    // Debug images contain Rc-backed bytecode; Arc is the VM's single-threaded
    // sharing handle, as it is in enter_generated.
    #[allow(clippy::arc_with_non_send_sync)]
    pub(super) fn command_callback(
        &mut self,
        words: &[VmObj],
        site: CodeAddress,
        name: &str,
        error_context: &str,
    ) -> Result<Result<VmObj, Completion>, VmError> {
        let depth = self.frames.len();
        let scope = self.frames[self.scope_index()].id;
        let namespace = self.frames[self.fi()].namespace.clone();
        #[cfg(feature = "debug")]
        let source = list_to_string(words)?;
        let result = (|| {
            #[cfg(feature = "debug")]
            {
                if self.source_images.contains_key(&site.code_id) {
                    self.enter_generated(
                        name,
                        source,
                        site,
                        namespace,
                        Some(scope),
                        FrameKind::Dynamic,
                        None,
                    )?;
                    self.frames.last_mut().unwrap().continuation = None;
                } else {
                    if !self.policy.allow_runtime_compile {
                        return Err(VmError::PolicyDenied {
                            action: "runtime_compile",
                            detail: "command callback compilation is disabled".into(),
                        });
                    }
                    let mode = match self.mode {
                        RuntimeMode::VmStrict => tcld_compile::CompileMode::VmStrict,
                        RuntimeMode::Hybrid => tcld_compile::CompileMode::Hybrid,
                    };
                    let image = tcld_compile::compile_source(
                        format!(".tcld-generated/{name}.tcl").into(),
                        source,
                        mode,
                    )
                    .map_err(|error| VmError::TclError {
                        message: error.reason,
                        error_info: None,
                        error_code: None,
                    })?;
                    let code = self.register_source_image(Arc::new(image), Some(site))?;
                    self.push_frame(code, FrameKind::Dynamic, None, &[], namespace, None)?;
                    self.frames.last_mut().unwrap().scope_parent = Some(scope);
                }
                self.command_callback_sites
                    .insert(self.frames.last().unwrap().code.id, site);
            }
            #[cfg(not(feature = "debug"))]
            {
                // Bytecode-only engines dispatch the already evaluated words.
                // Source compiler instrumentation needs the debug feature's
                // source maps and must never execute as plain TEBC.
                let _ = (site, name);
                let set = tcld_core::inst_table::DEFAULT_INSTRUCTION_SET;
                let invoke = set
                    .opcode_table
                    .iter()
                    .position(|op| *op == Opcode::InvokeStk4)
                    .unwrap() as u8;
                let done = set
                    .opcode_table
                    .iter()
                    .position(|op| *op == Opcode::Done)
                    .unwrap() as u8;
                let mut code_bytes = vec![invoke];
                code_bytes.extend_from_slice(&(words.len() as i32).to_be_bytes());
                code_bytes.push(done);
                let code = LoadedCode::from_bytecode(ByteCode {
                    header: None,
                    info: tcld_core::bytecode::ByteCodeInfo {
                        num_commands: 0,
                        num_src_bytes: 0,
                        num_code_bytes: code_bytes.len() as i32,
                        num_lit_objects: 0,
                        num_except_ranges: 0,
                        num_aux_data_items: 0,
                        num_cmd_loc_bytes: 0,
                        max_except_depth: 0,
                        max_stack_depth: words.len() as i32,
                        code_delta_size: 0,
                        code_length_size: 0,
                        src_delta_size: 0,
                        src_length_size: 0,
                    },
                    code_bytes,
                    cmd_code_ranges: vec![],
                    cmd_source_ranges: None,
                    literals: vec![],
                    exceptions: vec![],
                    auxiliaries: vec![],
                    proc_num_args: 0,
                    proc_num_locals: 0,
                    locals: vec![],
                })?;
                self.push_frame(code, FrameKind::Dynamic, None, &[], namespace, None)?;
                self.frames.last_mut().unwrap().scope_parent = Some(scope);
                self.frames.last_mut().unwrap().stack = words.to_vec();
            }
            #[cfg(feature = "debug")]
            let allow_pause = self.callbacks_can_pause();
            #[cfg(not(feature = "debug"))]
            let allow_pause = false;
            // Tcl uses Tcl_EvalObjv for these callbacks, rather than NRE
            // evaluation. A yield cannot escape this synchronous operation.
            #[cfg(feature = "debug")]
            {
                self.host_callback_depth += 1;
            }
            let outcome = self.eval_loop(depth, allow_pause);
            #[cfg(feature = "debug")]
            {
                self.host_callback_depth -= 1;
            }
            outcome
        })();
        if result.is_err() {
            self.frames.truncate(depth);
        }
        let mut completion = match result {
            Ok(LoopExit::Done(value)) => return Ok(Ok(value)),
            Ok(LoopExit::Completion(completion)) => completion,
            #[cfg(feature = "debug")]
            Ok(LoopExit::Paused) => {
                return Err(VmError::Internal(
                    "command callback escaped continuation".into(),
                ));
            }
            Err(VmError::TclError {
                message,
                error_info,
                error_code,
            }) => Completion::error(
                VmObj::from_str(&message),
                error_info.map(ByteString::from),
                error_code.map(|code| VmObj::from_str(&code)),
            ),
            Err(error) => return Err(error),
        };
        if completion.code == 1 {
            completion.append_error_context(error_context)?;
        }
        Ok(Err(completion))
    }
}
