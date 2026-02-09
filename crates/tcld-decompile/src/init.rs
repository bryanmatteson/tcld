use crate::decompiler::{DecompiledCommandSpan, DecompiledOutput, Decompiler};
use std::cell::RefCell;
use std::sync::Once;
use tcld_core::bytecode::{ByteCode, register_proc_body_renderer};

static INIT: Once = Once::new();

// Thread-local side-channel: after render_proc_body runs, the inner command
// spans are stashed here so that the caller (render_output_from_codes) can
// retrieve them and splice them into the outer span list.
thread_local! {
    static INNER_PROC_OUTPUT: RefCell<Option<InnerProcOutput>> = const { RefCell::new(None) };
    static RENDER_CONTEXTS: RefCell<Vec<RenderContext>> = const { RefCell::new(Vec::new()) };
}

pub(crate) struct InnerProcOutput {
    pub text: String,
    pub commands: Vec<DecompiledCommandSpan>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProcBodyRenderMode {
    Strict,
    Approximation,
}

struct RenderContext {
    mode: ProcBodyRenderMode,
    issues: Vec<String>,
}

pub(crate) fn with_proc_body_render_context<T>(
    mode: ProcBodyRenderMode,
    render: impl FnOnce() -> T,
) -> (T, Vec<String>) {
    RENDER_CONTEXTS.with(|contexts| {
        contexts.borrow_mut().push(RenderContext {
            mode,
            issues: Vec::new(),
        });
    });
    let output = render();
    let context = RENDER_CONTEXTS.with(|contexts| {
        contexts
            .borrow_mut()
            .pop()
            .expect("proc-body render context is balanced")
    });
    (output, context.issues)
}

fn current_render_mode() -> ProcBodyRenderMode {
    RENDER_CONTEXTS.with(|contexts| {
        contexts
            .borrow()
            .last()
            .map(|context| context.mode)
            .unwrap_or(ProcBodyRenderMode::Strict)
    })
}

fn record_render_issue(issue: String) {
    RENDER_CONTEXTS.with(|contexts| {
        if let Some(context) = contexts.borrow_mut().last_mut() {
            context.issues.push(issue);
        }
    });
}

/// Stash inner source and commands from a proc body decompilation.
fn stash_inner_output(output: DecompiledOutput) -> String {
    let text = output.text;
    INNER_PROC_OUTPUT.with(|cell| {
        *cell.borrow_mut() = Some(InnerProcOutput {
            text: text.clone(),
            commands: output.commands,
        });
    });
    text
}

/// Take stashed inner output (returns None if nothing was stashed).
pub(crate) fn take_inner_output() -> Option<InnerProcOutput> {
    INNER_PROC_OUTPUT.with(|cell| cell.borrow_mut().take())
}

pub(crate) fn clear_inner_output() {
    INNER_PROC_OUTPUT.with(|cell| {
        cell.borrow_mut().take();
    });
}

fn render_proc_body(bytecode: &ByteCode) -> String {
    let result = match current_render_mode() {
        ProcBodyRenderMode::Strict => {
            Decompiler::new_proc_body(bytecode.clone()).and_then(|d| d.get_output())
        }
        ProcBodyRenderMode::Approximation => {
            Decompiler::new_proc_body(bytecode.clone()).and_then(|d| {
                d.get_output_approximate_with_diagnostics().map(|result| {
                    for approximation in result.diagnostics.approximations {
                        let range = approximation
                            .bytecode_range
                            .map(|range| format!("inner bytecode {}..{}", range.start, range.end))
                            .unwrap_or_else(|| "nested procedure scope".to_string());
                        record_render_issue(format!(
                            "nested proc body approximation (args={}, locals={}, commands={}): {:?} at {range}: {}",
                            bytecode.proc_num_args,
                            bytecode.proc_num_locals,
                            bytecode.info.num_commands,
                            approximation.kind,
                            approximation.reason
                        ));
                    }
                    result.output
                })
            })
        }
    };
    let reason = match result {
        Ok(output) if !output.text.trim().is_empty() => {
            if output.commands.is_empty() {
                return output.text;
            }
            return stash_inner_output(output);
        }
        Ok(_) => "produced empty output".to_string(),
        Err(error) => error.to_string(),
    };

    record_render_issue(format!(
        "nested proc body could not be decompiled (args={}, locals={}, commands={}) [{}]",
        bytecode.proc_num_args, bytecode.proc_num_locals, bytecode.info.num_commands, reason
    ));
    // Approximation rendering retains a compact, visibly non-source placeholder.
    // Strict callers inspect the recorded issue and fail the outer operation.
    let summary = format!(
        "error {{tcld: unavailable nested bytecode(args={}, locals={}, commands={})}}",
        bytecode.proc_num_args, bytecode.proc_num_locals, bytecode.info.num_commands
    );
    format!("{{{summary}}}")
}

pub fn ensure_initialized() {
    INIT.call_once(|| {
        register_proc_body_renderer(render_proc_body);
    });
}
