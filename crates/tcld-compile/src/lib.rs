//! Deterministic Tcl source -> in-process debugger image compilation.
//!
//! This crate is the sole owner of source admission and verified source/PC
//! maps. It never evaluates Tcl and never shells out to a Tcl compiler.
//! Unknown commands in Hybrid mode are admitted only as opaque WorldHost leaf
//! calls: the image maps their outer source command and makes no claim about
//! implementation work inside the external command. Known Tcl commands/forms
//! that evaluate scripts are admitted only when their bodies can enter verified
//! images, including runtime-created images in Hybrid mode.

mod control;
mod dicts;
mod expression;
mod lists;
mod source;
mod strings;
mod words;

pub use strings::DEFAULT_TRIM_SET;

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sha2::{Digest, Sha256};
use tcld_core::bytecode::{
    AuxData, AuxDataType, AuxDataValue, ByteCode, ByteCodeInfo, CompiledLocal, ExceptionRange,
    ExceptionType, LiteralObject, LiteralObjectType, NewForeachInfo,
};
use tcld_core::inst_table::{Opcode, TCL_86_OPCODE_TABLE};
use tcld_core::source::SourceRef;

pub const DEBUG_IMAGE_SCHEMA: &str = "tcld.debug-image/v1";
pub const COMPILER_SEMANTICS: &str = "tcld.source-compiler/v1.17-try-tcl86";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompileMode {
    Hybrid,
    VmStrict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugTclAbi {
    Tcl86,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebugSourceIdentity {
    pub path: PathBuf,
    pub content: Arc<str>,
    pub content_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DebugCodeUnitKind {
    TopLevel,
    Procedure {
        qname: String,
        args_spec: String,
        /// Top-level source command whose execution activates this unit.
        declaration_span: Range<usize>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedCommandMap {
    /// Evaluated arguments are on the operand stack at this Nop boundary.
    pub trace_pc: u32,
    pub trace_end_pc: u32,
    pub trace_argv_count: usize,
    pub trace_inline: bool,
    pub trace_word_copies: Vec<(u32, usize)>,
    pub source_span: Range<usize>,
    pub start_line: u32,
    pub end_line: u32,
    /// Stable pause-before-execute instruction used for source breakpoints and
    /// source-step command boundaries.
    pub anchor_pc: u32,
    /// Full execution range for this command. Nested command ranges are
    /// strictly contained by their owning outer command.
    pub pc_range: Range<u32>,
    pub nesting_depth: u32,
}

#[derive(Clone)]
pub struct DebugCodeUnit {
    pub logical_id: String,
    pub kind: DebugCodeUnitKind,
    pub source_span: Range<usize>,
    pub bytecode: ByteCode,
    pub commands: Vec<VerifiedCommandMap>,
}

impl std::fmt::Debug for DebugCodeUnit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DebugCodeUnit")
            .field("logical_id", &self.logical_id)
            .field("kind", &self.kind)
            .field("source_span", &self.source_span)
            .field("commands", &self.commands)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
pub struct DebugImage {
    pub schema: &'static str,
    pub compiler_semantics: &'static str,
    pub image_digest: String,
    pub source: DebugSourceIdentity,
    pub abi: DebugTclAbi,
    pub mode: CompileMode,
    /// Runtime scripts in a procedure share Tcl's local compilation context.
    pub procedure_context: bool,
    pub units: Vec<DebugCodeUnit>,
    pub top_level_unit: usize,
}

impl DebugImage {
    pub fn top_level(&self) -> &DebugCodeUnit {
        &self.units[self.top_level_unit]
    }

    pub fn unit(&self, logical_id: &str) -> Option<&DebugCodeUnit> {
        self.units.iter().find(|unit| unit.logical_id == logical_id)
    }

    pub fn verify(&self) -> Result<(), String> {
        if self.schema != DEBUG_IMAGE_SCHEMA || self.compiler_semantics != COMPILER_SEMANTICS {
            return Err("debug image schema/compiler semantics mismatch".into());
        }
        if self.units.is_empty() || self.top_level_unit >= self.units.len() {
            return Err("debug image has no top-level unit".into());
        }
        if !matches!(
            self.units[self.top_level_unit].kind,
            DebugCodeUnitKind::TopLevel
        ) || self
            .units
            .iter()
            .filter(|unit| matches!(unit.kind, DebugCodeUnitKind::TopLevel))
            .count()
            != 1
        {
            return Err("debug image top-level ownership mismatch".into());
        }
        if self.source.content_sha256 != hex_digest(self.source.content.as_bytes()) {
            return Err("debug image source content digest mismatch".into());
        }
        if self.image_digest
            != image_identity_digest(
                self.schema,
                self.compiler_semantics,
                self.mode,
                &self.source.content,
                self.procedure_context,
            )
        {
            return Err("debug image identity digest mismatch".into());
        }
        let mut ids = std::collections::BTreeSet::new();
        for unit in &self.units {
            if !ids.insert(unit.logical_id.as_str()) {
                return Err(format!("duplicate logical code unit {}", unit.logical_id));
            }
            let code_len = unit.bytecode.code_bytes.len() as u32;
            if unit.commands.len() != unit.bytecode.info.num_commands as usize
                || unit.commands.len() != unit.bytecode.cmd_code_ranges.len()
                || unit
                    .bytecode
                    .cmd_source_ranges
                    .as_ref()
                    .is_none_or(|ranges| ranges.len() != unit.commands.len())
            {
                return Err(format!(
                    "command map/bytecode metadata count mismatch in {}",
                    unit.logical_id
                ));
            }
            let source_ranges = unit.bytecode.cmd_source_ranges.as_ref().unwrap();
            for (index, map) in unit.commands.iter().enumerate() {
                let code_range = &unit.bytecode.cmd_code_ranges[index];
                let source_range = &source_ranges[index];
                if code_range.start < 0
                    || code_range.end < 0
                    || map.pc_range != (code_range.start as u32..code_range.end as u32)
                    || source_range.start < 0
                    || source_range.end < 0
                    || map.source_span != (source_range.start as usize..source_range.end as usize)
                {
                    return Err(format!(
                        "command map/bytecode metadata disagreement in {}",
                        unit.logical_id
                    ));
                }
            }
            let mut anchors = std::collections::BTreeSet::new();
            for map in &unit.commands {
                let range = &map.pc_range;
                if range.start >= range.end || range.end > code_len {
                    return Err(format!(
                        "invalid PC range {:?} in {}",
                        range, unit.logical_id
                    ));
                }
                if map.anchor_pc != range.start
                    || !anchors.insert(map.anchor_pc)
                    || unit.bytecode.code_bytes.get(map.anchor_pc as usize)
                        != Some(&(Opcode::Nop as u8))
                {
                    return Err(format!(
                        "invalid or duplicate command anchor {} in {}",
                        map.anchor_pc, unit.logical_id
                    ));
                }
                if map.trace_pc <= map.anchor_pc
                    || map.trace_end_pc <= map.trace_pc
                    || map.trace_end_pc >= range.end
                    || map.trace_argv_count == 0
                    || [map.trace_pc, map.trace_end_pc].iter().any(|pc| {
                        unit.bytecode.code_bytes.get(*pc as usize) != Some(&(Opcode::Nop as u8))
                    })
                    || map.trace_word_copies.iter().any(|(pc, index)| {
                        *pc <= map.trace_pc
                            || *pc >= map.trace_end_pc
                            || *index >= map.trace_argv_count
                            || unit.bytecode.code_bytes.get(*pc as usize)
                                != Some(&(Opcode::Nop as u8))
                    })
                {
                    return Err(format!(
                        "invalid execution trace boundaries in {}",
                        unit.logical_id
                    ));
                }
            }
            for left_index in 0..unit.commands.len() {
                for right_index in left_index + 1..unit.commands.len() {
                    let left = &unit.commands[left_index];
                    let right = &unit.commands[right_index];
                    if left.pc_range.end <= right.pc_range.start
                        || right.pc_range.end <= left.pc_range.start
                    {
                        continue;
                    }
                    let left_contains = left.pc_range.start < right.pc_range.start
                        && left.pc_range.end > right.pc_range.end;
                    let right_contains = right.pc_range.start < left.pc_range.start
                        && right.pc_range.end > left.pc_range.end;
                    let source_left_contains = left.source_span.start <= right.source_span.start
                        && left.source_span.end >= right.source_span.end
                        && left.nesting_depth < right.nesting_depth;
                    let source_right_contains = right.source_span.start <= left.source_span.start
                        && right.source_span.end >= left.source_span.end
                        && right.nesting_depth < left.nesting_depth;
                    if !((left_contains && source_left_contains)
                        || (right_contains && source_right_contains))
                    {
                        return Err(format!(
                            "command PC overlap is not source-nested in {}: {left:?}, {right:?}",
                            unit.logical_id
                        ));
                    }
                }
            }
            for map in &unit.commands {
                if map.source_span.start >= map.source_span.end
                    || map.source_span.end > self.source.content.len()
                    || map.source_span.start < unit.source_span.start
                    || map.source_span.end > unit.source_span.end
                {
                    return Err(format!("invalid source span in {}", unit.logical_id));
                }
                let words = scan_words(&self.source.content[map.source_span.clone()])?;
                if words.len() != map.trace_argv_count {
                    return Err(format!(
                        "execution argument/source disagreement in {}",
                        unit.logical_id
                    ));
                }
                let (start_line, _) = line_column(&self.source.content, map.source_span.start);
                let (end_line, _) =
                    line_column(&self.source.content, map.source_span.end.saturating_sub(1));
                if map.start_line != start_line || map.end_line != end_line {
                    return Err(format!("source line map mismatch in {}", unit.logical_id));
                }
            }
            let unique_spans = unit
                .commands
                .iter()
                .map(|map| (map.source_span.start, map.source_span.end))
                .collect::<std::collections::BTreeSet<_>>();
            if unique_spans.len() != unit.commands.len() {
                return Err(format!(
                    "source command mapped more than once in {}",
                    unit.logical_id
                ));
            }
            if !unit.commands.is_empty() {
                for instruction in unit
                    .bytecode
                    .get_instructions_raw()
                    .map_err(|error| format!("invalid bytecode in {}: {error}", unit.logical_id))?
                {
                    let pc = u32::try_from(instruction.location)
                        .map_err(|_| format!("negative instruction PC in {}", unit.logical_id))?;
                    if !unit.commands.iter().any(|map| map.pc_range.contains(&pc)) {
                        return Err(format!(
                            "instruction PC {pc} has no source command in {}",
                            unit.logical_id
                        ));
                    }
                }
            }
        }
        let spans = source::command_spans(&self.source.content, self.procedure_context)
            .map_err(|error| format!("debug image source cannot be reparsed: {error}"))?;
        let proc_body_ranges = self
            .units
            .iter()
            .filter_map(|unit| match unit.kind {
                DebugCodeUnitKind::Procedure { .. } => Some(unit.source_span.clone()),
                DebugCodeUnitKind::TopLevel => None,
            })
            .collect::<Vec<_>>();
        let proc_names = self
            .units
            .iter()
            .filter_map(|unit| match &unit.kind {
                DebugCodeUnitKind::Procedure { qname, .. } => Some(qname.as_str()),
                DebugCodeUnitKind::TopLevel => None,
            })
            .collect::<std::collections::BTreeSet<_>>();
        let declaration_spans = self
            .units
            .iter()
            .filter_map(|unit| match &unit.kind {
                DebugCodeUnitKind::Procedure {
                    declaration_span, ..
                } => Some((declaration_span.start, declaration_span.end)),
                DebugCodeUnitKind::TopLevel => None,
            })
            .collect::<std::collections::BTreeSet<_>>();
        for span in &spans {
            let text = &self.source.content[span.start_byte..span.end_byte];
            let words = scan_words(text)
                .map_err(|reason| format!("invalid mapped command words: {reason}"))?;
            let name = words
                .first()
                .ok_or_else(|| "empty mapped source command".to_string())
                .and_then(|word| word_literal(text, word.clone()))
                .unwrap_or_default();
            if name == "proc" && declaration_spans.contains(&(span.start_byte, span.end_byte)) {
                continue;
            }
            let qname = if name.starts_with("::") {
                name.clone()
            } else {
                format!("::{name}")
            };
            match command_effect(&name, text, &words, proc_names.contains(qname.as_str())) {
                CommandEffect::TclScriptOutsideImage(reason) => return Err(reason),
                CommandEffect::OpaqueWorldHost
                    if self.mode == CompileMode::VmStrict && is_tcl_builtin_command(&name) =>
                {
                    return Err(format!(
                        "command {name:?} requires WorldHost and is not admitted by vm-strict"
                    ));
                }
                CommandEffect::CompilerOwned | CommandEffect::OpaqueWorldHost => {}
            }
        }
        verify_complete_source_maps(&spans, &proc_body_ranges, &self.units, self.top_level_unit)?;
        for unit in &self.units {
            let DebugCodeUnitKind::Procedure {
                declaration_span, ..
            } = &unit.kind
            else {
                continue;
            };
            if !self.units[self.top_level_unit]
                .commands
                .iter()
                .any(|map| map.source_span == *declaration_span)
            {
                return Err(format!(
                    "procedure {} has no declaration activation map",
                    unit.logical_id
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "source_compile_failed: {path}:{line}:{column}: {reason}; command={command:?}; use --backend=host"
)]
pub struct CompileDiagnostic {
    pub path: PathBuf,
    pub span: Range<usize>,
    pub line: u32,
    pub column: u32,
    pub command: String,
    pub reason: String,
}

pub fn compile_path(path: &Path, mode: CompileMode) -> Result<DebugImage, CompileDiagnostic> {
    let canonical = std::fs::canonicalize(path)
        .map_err(|error| diagnostic(path, "", 0..0, "", format!("cannot read source: {error}")))?;
    let source = std::fs::read_to_string(&canonical).map_err(|error| {
        diagnostic(
            &canonical,
            "",
            0..0,
            "",
            format!("source is not valid UTF-8: {error}"),
        )
    })?;
    compile_source(canonical, source, mode)
}

pub fn compile_source(
    path: PathBuf,
    source: String,
    mode: CompileMode,
) -> Result<DebugImage, CompileDiagnostic> {
    compile_source_in_context(path, source, mode, false)
}

/// Compile a runtime script without wrapping or changing its source. A local
/// context permits Tcl's procedure-only loop instructions while retaining the
/// caller's variable scope and the exact source map.
pub fn compile_source_in_context(
    path: PathBuf,
    source: String,
    mode: CompileMode,
    procedure_context: bool,
) -> Result<DebugImage, CompileDiagnostic> {
    let spans = source::command_spans(&source, procedure_context)
        .map_err(|reason| diagnostic(&path, &source, 0..source.len().min(1), "", reason))?;
    let direct_top = direct_commands(&spans, 0..source.len());
    let mut proc_decls = Vec::new();
    let mut proc_signatures = std::collections::BTreeMap::new();
    for span in &direct_top {
        if span.name.as_deref() != Some("proc") {
            continue;
        }
        let decl = match parse_proc_decl(&source, span.start_byte..span.end_byte) {
            Ok(decl) => decl,
            Err(reason) => {
                let text = &source[span.start_byte..span.end_byte];
                let words = scan_words(text).unwrap_or_default();
                if words.len() == 4
                    && words.iter().all(|w| word_literal(text, w.clone()).is_ok())
                    && text[words[2].clone()].starts_with('{')
                    && text[words[3].clone()].starts_with('{')
                {
                    return Err(diagnostic(
                        &path,
                        &source,
                        span.start_byte..span.end_byte,
                        text,
                        reason,
                    ));
                }
                continue;
            }
        };
        let signature = ProcSignature::from_args(&decl.args);
        if proc_signatures
            .insert(decl.qname.clone(), signature)
            .is_some()
        {
            return Err(diagnostic(
                &path,
                &source,
                span.start_byte..span.end_byte,
                &span.text,
                format!("duplicate static proc declaration for {}", decl.qname),
            ));
        }
        proc_decls.push(decl);
    }

    let proc_body_ranges = proc_decls
        .iter()
        .map(|decl| decl.body_span.clone())
        .collect::<Vec<_>>();
    let mut units = Vec::new();
    for decl in proc_decls {
        let declaration_span = spans
            .iter()
            .find(|span| {
                span.name.as_deref() == Some("proc")
                    && span.start_byte < decl.body_span.start
                    && span.end_byte > decl.body_span.end
            })
            .map(|span| span.start_byte..span.end_byte)
            .ok_or_else(|| {
                diagnostic(
                    &path,
                    &source,
                    decl.body_span.clone(),
                    "proc",
                    format!("missing declaration command for {}", decl.qname),
                )
            })?;
        let body_commands = direct_commands(&spans, decl.body_span.clone());
        let bytecode = Compiler::new(&path, &source, mode, &proc_signatures, false).compile_unit(
            &body_commands,
            decl.body_span.clone(),
            Some(&decl.args_spec),
        )?;
        let logical_id = format!("proc:{}", decl.qname.trim_start_matches("::"));
        units.push(DebugCodeUnit {
            logical_id,
            kind: DebugCodeUnitKind::Procedure {
                qname: decl.qname,
                args_spec: decl.args_spec,
                declaration_span,
            },
            source_span: decl.body_span,
            commands: bytecode.maps,
            bytecode: bytecode.bytecode,
        });
    }

    let mut compiler = Compiler::new(&path, &source, mode, &proc_signatures, true);
    compiler.procedure_context = procedure_context;
    let top = compiler.compile_unit(&direct_top, 0..source.len(), None)?;
    let top_index = units.len();
    units.push(DebugCodeUnit {
        logical_id: "file:toplevel".into(),
        kind: DebugCodeUnitKind::TopLevel,
        source_span: 0..source.len(),
        commands: top.maps,
        bytecode: top.bytecode,
    });

    verify_complete_source_maps(&spans, &proc_body_ranges, &units, top_index)
        .map_err(|reason| diagnostic(&path, &source, 0..source.len().min(1), "", reason))?;

    let content_sha256 = hex_digest(source.as_bytes());
    let image = DebugImage {
        schema: DEBUG_IMAGE_SCHEMA,
        compiler_semantics: COMPILER_SEMANTICS,
        image_digest: image_identity_digest(
            DEBUG_IMAGE_SCHEMA,
            COMPILER_SEMANTICS,
            mode,
            &source,
            procedure_context,
        ),
        source: DebugSourceIdentity {
            path,
            content: Arc::from(source),
            content_sha256,
        },
        abi: DebugTclAbi::Tcl86,
        mode,
        procedure_context,
        units,
        top_level_unit: top_index,
    };
    image.verify().map_err(|reason| {
        diagnostic(
            &image.source.path,
            &image.source.content,
            0..image.source.content.len().min(1),
            "",
            reason,
        )
    })?;
    Ok(image)
}

fn direct_commands(
    spans: &[tcld_syntax::TclCommandSpan],
    unit: Range<usize>,
) -> Vec<&tcld_syntax::TclCommandSpan> {
    spans
        .iter()
        .filter(|candidate| {
            candidate.start_byte >= unit.start
                && candidate.end_byte <= unit.end
                && !spans.iter().any(|parent| {
                    parent.start_byte >= unit.start
                        && parent.end_byte <= unit.end
                        && parent.start_byte <= candidate.start_byte
                        && parent.end_byte >= candidate.end_byte
                        && (parent.start_byte < candidate.start_byte
                            || parent.end_byte > candidate.end_byte)
                })
        })
        .collect()
}

/// Prove that the compiler did not silently drop, duplicate, or assign an
/// admitted parser command to the wrong runtime code unit. Procedure bodies
/// are separate runtime units; their surrounding `proc` declaration remains
/// a top-level command boundary.
fn verify_complete_source_maps(
    spans: &[tcld_syntax::TclCommandSpan],
    proc_body_ranges: &[Range<usize>],
    units: &[DebugCodeUnit],
    top_index: usize,
) -> Result<(), String> {
    for (index, unit) in units.iter().enumerate() {
        let selected = spans
            .iter()
            .filter(|span| {
                if index == top_index {
                    !proc_body_ranges
                        .iter()
                        .any(|body| span.start_byte >= body.start && span.end_byte <= body.end)
                } else {
                    span.start_byte >= unit.source_span.start
                        && span.end_byte <= unit.source_span.end
                }
            })
            .collect::<Vec<_>>();
        let mut expected = selected
            .iter()
            .map(|span| {
                let nesting_depth = selected
                    .iter()
                    .filter(|parent| {
                        parent.start_byte <= span.start_byte
                            && parent.end_byte >= span.end_byte
                            && (parent.start_byte < span.start_byte
                                || parent.end_byte > span.end_byte)
                    })
                    .count() as u32;
                (span.start_byte, span.end_byte, nesting_depth)
            })
            .collect::<Vec<_>>();
        let mut actual = unit
            .commands
            .iter()
            .map(|map| {
                (
                    map.source_span.start,
                    map.source_span.end,
                    map.nesting_depth,
                )
            })
            .collect::<Vec<_>>();
        expected.sort_unstable();
        actual.sort_unstable();
        if actual != expected {
            return Err(format!(
                "incomplete source command map in {}: expected {expected:?}, mapped {actual:?}",
                unit.logical_id
            ));
        }
    }
    Ok(())
}

struct ProcDecl {
    qname: String,
    args_spec: String,
    args: Vec<ProcArg>,
    body_span: Range<usize>,
}

/// One formal parameter: its name and, for a `{name default}` pair, the
/// literal value Tcl binds when a call leaves it out.
struct ProcArg {
    name: String,
    default: Option<String>,
}

#[derive(Clone, Copy)]
struct ProcSignature {
    /// Arguments every call supplies. Tcl binds positionally, so a parameter
    /// without a default makes every parameter before it required too.
    required: usize,
    /// Parameters before a final variadic `args`.
    positional: usize,
    variadic: bool,
}
impl ProcSignature {
    fn from_args(args: &[ProcArg]) -> Self {
        let variadic = args.last().is_some_and(|arg| arg.name == "args");
        let positional = &args[..args.len() - usize::from(variadic)];
        Self {
            required: positional
                .iter()
                .rposition(|arg| arg.default.is_none())
                .map_or(0, |index| index + 1),
            positional: positional.len(),
            variadic,
        }
    }

    fn accepts(&self, supplied: usize) -> bool {
        supplied >= self.required && (self.variadic || supplied <= self.positional)
    }

    fn expected(&self) -> String {
        if self.variadic {
            format!("{} or more", self.required)
        } else if self.required == self.positional {
            self.required.to_string()
        } else {
            format!("{} to {}", self.required, self.positional)
        }
    }
}

/// Original (possibly relative) command name, before namespace qualification.
pub fn procedure_declaration_name(source: &str, span: Range<usize>) -> Result<String, String> {
    let text = source
        .get(span)
        .ok_or("invalid procedure declaration span")?;
    let words = scan_words(text)?;
    word_literal(text, words.get(1).cloned().ok_or("missing procedure name")?)
}

/// The body of a procedure declaration as `info body` reports it.
pub fn procedure_declaration_body(source: &str, span: Range<usize>) -> Result<String, String> {
    let text = source
        .get(span)
        .ok_or("invalid procedure declaration span")?;
    let words = scan_words(text)?;
    word_literal(text, words.get(3).cloned().ok_or("missing procedure body")?)
}

fn parse_proc_decl(source: &str, span: Range<usize>) -> Result<ProcDecl, String> {
    let text = &source[span.clone()];
    let words =
        scan_words(text).map_err(|reason| format!("unsupported proc declaration: {reason}"))?;
    if words.len() != 4 || word_literal(text, words[0].clone())? != "proc" {
        return Err("proc must have exactly name, argument list, and braced body".into());
    }
    let mut qname = word_literal(text, words[1].clone())?;
    if qname.is_empty() || qname.contains('$') || qname.contains('[') {
        return Err("proc name must be static".into());
    }
    if !qname.starts_with("::") {
        qname = format!("::{qname}");
    }
    let args_spec = braced_literal(text, words[2].clone())?;
    let args = parse_proc_args(&args_spec)?;
    let body_local = words[3].clone();
    let _ = braced_literal(text, body_local.clone())?;
    Ok(ProcDecl {
        qname,
        args_spec,
        args,
        body_span: (span.start + body_local.start + 1)..(span.start + body_local.end - 1),
    })
}

struct CompiledUnit {
    bytecode: ByteCode,
    maps: Vec<VerifiedCommandMap>,
}

struct Compiler<'a> {
    path: &'a Path,
    source: &'a str,
    mode: CompileMode,
    proc_signatures: &'a std::collections::BTreeMap<String, ProcSignature>,
    code: Vec<u8>,
    literals: Vec<LiteralObject>,
    maps: Vec<VerifiedCommandMap>,
    static_declarations: bool,
    procedure_context: bool,
    return_context: Vec<ExceptionType>,
    captured_words: Vec<(Range<usize>, u32, usize)>,
    word_copies: Vec<(u32, u32, usize)>,
    locals: Vec<String>,
    exceptions: Vec<ExceptionRange>,
    /// Extra exception nesting for ranges a command opens around a body it
    /// compiles, beyond the body's source nesting.
    exception_bias: u32,
    auxiliaries: Vec<AuxData>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CommandEffect {
    CompilerOwned,
    /// An external command invoked atomically at one mapped outer boundary.
    OpaqueWorldHost,
    /// A Tcl command/form that would execute source not owned by this image.
    TclScriptOutsideImage(String),
}

fn command_effect(
    name: &str,
    text: &str,
    words: &[Range<usize>],
    is_static_proc: bool,
) -> CommandEffect {
    let name = if name.starts_with("::") {
        name.trim_start_matches(':')
    } else {
        name
    };
    if name.is_empty() {
        return CommandEffect::CompilerOwned;
    }
    if lists::is_native_command(name) {
        return CommandEffect::CompilerOwned;
    }
    if matches!(
        name,
        "set"
            | "return"
            | "expr"
            | "append"
            | "lappend"
            | "incr"
            | "unset"
            | "global"
            | "variable"
            | "if"
            | "while"
            | "for"
            | "foreach"
            | "catch"
            | "try"
            | "switch"
            | "lmap"
            | "subst"
            | "time"
            | "break"
            | "continue"
            | "eval"
            | "proc"
            | "source"
            | "uplevel"
            | "upvar"
            | "apply"
            | "after"
            | "update"
            | "vwait"
            | "trace"
            | "fileevent"
            | "coroutine"
            | "yield"
            | "yieldto"
            | "tailcall"
            | "::tcld::debug_event_ready"
            | "::tcld::debug_event_dispatch"
            | "tcld::debug_event_ready"
            | "tcld::debug_event_dispatch"
    ) || is_static_proc
    {
        return CommandEffect::CompilerOwned;
    }
    let rejects_script = |form: &str| {
        CommandEffect::TclScriptOutsideImage(format!(
            "{form} evaluates or schedules Tcl script outside the debug image; V1 cannot guarantee canonical stepping"
        ))
    };
    match name {
        "exit" => CommandEffect::TclScriptOutsideImage(
            "exit would terminate the debugger host outside the debug image; V1 does not admit it"
                .into(),
        ),
        "namespace" => match static_subcommand(text, words) {
            Some(subcommand) if matches!(subcommand.as_str(), "unknown" | "ensemble") => {
                rejects_script(&format!("namespace {subcommand}"))
            }
            Some(subcommand)
                if matches!(subcommand.as_str(), "eval" | "inscope" | "code" | "current") =>
            {
                CommandEffect::CompilerOwned
            }
            Some(_) => CommandEffect::OpaqueWorldHost,
            None => CommandEffect::CompilerOwned,
        },
        "interp" => match static_subcommand(text, words) {
            Some(subcommand)
                if matches!(
                    subcommand.as_str(),
                    "eval" | "alias" | "bgerror" | "limit" | "create" | "invokehidden"
                ) =>
            {
                rejects_script(&format!("interp {subcommand}"))
            }
            Some(_) => CommandEffect::OpaqueWorldHost,
            None => rejects_script("interp with a dynamic subcommand"),
        },
        // The VM runs every `dict` form but `info`, which reports Tcl's own
        // hash table; the forms that evaluate a script are deferred.
        "dict" => match static_subcommand(text, words)
            .as_deref()
            .and_then(dict_subcommand)
        {
            Some("info") => CommandEffect::OpaqueWorldHost,
            _ => CommandEffect::CompilerOwned,
        },
        "bind" | "oo::define" | "oo::objdefine" | "unknown" | "auto_load" | "auto_import"
        | "timerate" | "history" => rejects_script(name),
        "oo::class" => match static_subcommand(text, words) {
            Some(subcommand) if matches!(subcommand.as_str(), "create" | "new") => {
                rejects_script(&format!("oo::class {subcommand}"))
            }
            Some(_) => CommandEffect::OpaqueWorldHost,
            None => rejects_script("oo::class with a dynamic subcommand"),
        },
        "oo::object" => match static_subcommand(text, words) {
            Some(subcommand) if matches!(subcommand.as_str(), "create" | "new") => {
                rejects_script(&format!("oo::object {subcommand}"))
            }
            Some(_) => CommandEffect::OpaqueWorldHost,
            None => rejects_script("oo::object with a dynamic subcommand"),
        },
        "package" => match static_subcommand(text, words) {
            Some(subcommand)
                if matches!(subcommand.as_str(), "require" | "ifneeded" | "unknown") =>
            {
                rejects_script(&format!("package {subcommand}"))
            }
            Some(_) => CommandEffect::OpaqueWorldHost,
            None => rejects_script("package with a dynamic subcommand"),
        },
        "socket" => {
            if words
                .iter()
                .skip(1)
                .any(|word| word_literal(text, word.clone()).ok().as_deref() == Some("-server"))
            {
                rejects_script("socket -server")
            } else {
                CommandEffect::OpaqueWorldHost
            }
        }
        "lsort" => CommandEffect::CompilerOwned,
        // vm-strict writes stdout and stderr itself; any other channel is
        // Tcl's.
        "puts" => match puts_channel(text, words) {
            Some(channel) if channel != "stdout" && channel != "stderr" => {
                CommandEffect::OpaqueWorldHost
            }
            _ => CommandEffect::CompilerOwned,
        },
        "info" => match static_subcommand(text, words)
            .as_deref()
            .and_then(info_subcommand)
        {
            Some("exists" | "commands" | "procs" | "body" | "args" | "level") => {
                CommandEffect::CompilerOwned
            }
            _ => CommandEffect::OpaqueWorldHost,
        },
        "rename" => CommandEffect::CompilerOwned,
        "lsearch" | "regexp" | "regsub" | "string" => CommandEffect::CompilerOwned,
        "clock" if clock_reading(text, words).is_some() => CommandEffect::CompilerOwned,
        "fcopy" => {
            if words
                .iter()
                .skip(1)
                .any(|word| word_literal(text, word.clone()).ok().as_deref() == Some("-command"))
            {
                rejects_script(&format!("{name} -command"))
            } else {
                CommandEffect::OpaqueWorldHost
            }
        }
        "chan" => match static_subcommand(text, words) {
            Some(subcommand) if subcommand == "event" => CommandEffect::CompilerOwned,
            Some(_) => CommandEffect::OpaqueWorldHost,
            None => CommandEffect::CompilerOwned,
        },
        _ => CommandEffect::OpaqueWorldHost,
    }
}

/// The channel a `puts` command writes to (`Tcl_PutsObjCmd`): `stdout` when
/// it names none, and `None` when a computed word decides or the command is
/// a usage error.
fn puts_channel(text: &str, words: &[Range<usize>]) -> Option<String> {
    if words.iter().any(|word| expanded_word(&text[word.clone()])) {
        return None;
    }
    let literal = |index: usize| word_literal(text, words[index].clone()).ok();
    match words.len() {
        2 => Some("stdout".into()),
        3 => match literal(1)? {
            first if first == "-nonewline" => Some("stdout".into()),
            channel => Some(channel),
        },
        4 if literal(1)? == "-nonewline" => literal(2),
        _ => None,
    }
}

/// The `info` subcommand a word names: an exact subcommand or a unique prefix
/// (`defaultInfoMap`).
fn info_subcommand(name: &str) -> Option<&'static str> {
    unique_prefix(
        &[
            "args",
            "body",
            "cmdcount",
            "cmdtype",
            "commands",
            "complete",
            "constant",
            "consts",
            "coroutine",
            "default",
            "errorstack",
            "exists",
            "frame",
            "functions",
            "globals",
            "hostname",
            "level",
            "library",
            "loaded",
            "locals",
            "nameofexecutable",
            "patchlevel",
            "procs",
            "script",
            "sharedlibextension",
            "tclversion",
            "vars",
        ],
        name,
    )
}

/// Tcl's dictionary ensemble accepts unambiguous subcommand prefixes. Use
/// the complete upstream subcommand set before classifying executable bodies.
pub fn dict_subcommand(name: &str) -> Option<&'static str> {
    let commands = [
        "append",
        "create",
        "exists",
        "filter",
        "for",
        "get",
        "getdef",
        "getwithdefault",
        "incr",
        "info",
        "keys",
        "lappend",
        "map",
        "merge",
        "remove",
        "replace",
        "set",
        "size",
        "unset",
        "update",
        "values",
        "with",
    ];
    unique_prefix(&commands, name)
}

/// Tcl's `Tcl_GetIndexFromObj` rule: an exact entry, or a prefix of exactly
/// one entry.
fn unique_prefix(table: &[&'static str], name: &str) -> Option<&'static str> {
    if let Some(exact) = table.iter().find(|s| **s == name) {
        return Some(exact);
    }
    let mut matches = table
        .iter()
        .filter(|s| !name.is_empty() && s.starts_with(name));
    let first = matches.next()?;
    if matches.next().is_none() {
        Some(first)
    } else {
        None
    }
}

/// The `clockRead` selector for a `clock` reading Tcl compiles
/// (`TclCompileClockReadingCmd`, `TclCompileClockClicksCmd`).
fn clock_reading(text: &str, words: &[Range<usize>]) -> Option<u32> {
    if words.iter().any(|word| expanded_word(&text[word.clone()])) {
        return None;
    }
    match (static_subcommand(text, words)?.as_str(), words.len()) {
        ("clicks", 2) => Some(0),
        ("microseconds", 2) => Some(1),
        ("milliseconds", 2) => Some(2),
        ("seconds", 2) => Some(3),
        ("clicks", 3) => {
            let flag = word_literal(text, words[2].clone()).ok()?;
            if !(4..=13).contains(&flag.len()) {
                None
            } else if "-microseconds".starts_with(&flag) {
                Some(1)
            } else if "-milliseconds".starts_with(&flag) {
                Some(2)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Tcl's flag for an unnamed compiler temporary in a local variable table.
pub const VAR_TEMPORARY: i32 = 0x200;

/// Tcl 9's global commands (`info commands`), with the library procedures it
/// loads on demand, sorted.
const TCL_BUILTIN_COMMANDS: &[&str] = &[
    "after",
    "append",
    "apply",
    "array",
    "auto_execok",
    "auto_import",
    "auto_load",
    "auto_load_index",
    "auto_mkindex",
    "auto_mkindex_old",
    "auto_qualify",
    "auto_reset",
    "binary",
    "break",
    "catch",
    "cd",
    "chan",
    "clock",
    "close",
    "concat",
    "const",
    "continue",
    "coroinject",
    "coroprobe",
    "coroutine",
    "dict",
    "encoding",
    "eof",
    "error",
    "eval",
    "exec",
    "exit",
    "expr",
    "fblocked",
    "fconfigure",
    "fcopy",
    "file",
    "fileevent",
    "flush",
    "for",
    "foreach",
    "foreachLine",
    "format",
    "fpclassify",
    "gets",
    "glob",
    "global",
    "history",
    "if",
    "incr",
    "info",
    "interp",
    "join",
    "lappend",
    "lassign",
    "ledit",
    "lindex",
    "linsert",
    "list",
    "llength",
    "lmap",
    "load",
    "lpop",
    "lrange",
    "lremove",
    "lrepeat",
    "lreplace",
    "lreverse",
    "lsearch",
    "lseq",
    "lset",
    "lsort",
    "namespace",
    "open",
    "package",
    "parray",
    "pid",
    "pkg_mkIndex",
    "proc",
    "puts",
    "pwd",
    "read",
    "readFile",
    "regexp",
    "regsub",
    "rename",
    "return",
    "scan",
    "seek",
    "set",
    "socket",
    "source",
    "split",
    "string",
    "subst",
    "switch",
    "tailcall",
    "tclLog",
    "tclPkgSetup",
    "tclPkgUnknown",
    "tcl_endOfWord",
    "tcl_findLibrary",
    "tcl_startOfNextWord",
    "tcl_startOfPreviousWord",
    "tcl_wordBreakAfter",
    "tcl_wordBreakBefore",
    "tell",
    "throw",
    "time",
    "timerate",
    "trace",
    "try",
    "unknown",
    "unload",
    "unset",
    "update",
    "uplevel",
    "upvar",
    "variable",
    "vwait",
    "while",
    "writeFile",
    "yield",
    "yieldto",
    "zipfs",
    "zlib",
];

/// Whether `name` is a command the Tcl host provides. In vm-strict any other
/// command outside the image may be a procedure the program creates at
/// runtime, so it is resolved when it runs.
pub fn is_tcl_builtin_command(name: &str) -> bool {
    let name = name.strip_prefix("::").unwrap_or(name);
    TCL_BUILTIN_COMMANDS.binary_search(&name).is_ok()
        || ["tcl::", "oo::", "zlib::"]
            .iter()
            .any(|namespace| name.starts_with(namespace))
}

fn static_subcommand(text: &str, words: &[Range<usize>]) -> Option<String> {
    words
        .get(1)
        .and_then(|word| word_literal(text, word.clone()).ok())
}

impl<'a> Compiler<'a> {
    fn new(
        path: &'a Path,
        source: &'a str,
        mode: CompileMode,
        proc_signatures: &'a std::collections::BTreeMap<String, ProcSignature>,
        static_declarations: bool,
    ) -> Self {
        Self {
            path,
            source,
            mode,
            proc_signatures,
            code: Vec::new(),
            literals: Vec::new(),
            maps: Vec::new(),
            static_declarations,
            procedure_context: false,
            return_context: Vec::new(),
            captured_words: Vec::new(),
            word_copies: Vec::new(),
            locals: Vec::new(),
            exceptions: Vec::new(),
            exception_bias: 0,
            auxiliaries: Vec::new(),
        }
    }

    fn compile_unit(
        mut self,
        commands: &[&tcld_syntax::TclCommandSpan],
        unit_span: Range<usize>,
        args_spec: Option<&str>,
    ) -> Result<CompiledUnit, CompileDiagnostic> {
        self.procedure_context |= args_spec.is_some();
        let (names, defaults): (Vec<_>, Vec<_>) = args_spec
            .map(parse_proc_args)
            .transpose()
            .map_err(|reason| {
                diagnostic(self.path, self.source, unit_span.clone(), "proc", reason)
            })?
            .unwrap_or_default()
            .into_iter()
            .map(|arg| (arg.name, arg.default))
            .unzip();
        self.locals = names;
        let argument_count = self.locals.len();
        for (index, command) in commands.iter().enumerate() {
            self.compile_command(command.start_byte..command.end_byte, 0)?;
            if index + 1 != commands.len() {
                self.emit(Opcode::Pop);
                self.maps
                    .last_mut()
                    .expect("compiled command map")
                    .pc_range
                    .end = self.code.len() as u32;
            }
        }
        if commands.is_empty() {
            self.push_literal("");
        }
        self.emit(Opcode::Done);
        if !commands.is_empty() {
            self.maps
                .last_mut()
                .expect("final compiled command map")
                .pc_range
                .end = self.code.len() as u32;
        }
        let locals =
            self.locals
                .into_iter()
                .enumerate()
                .map(|(index, name)| (index, name.is_empty(), name))
                .map(|(index, name_is_temporary, name)| CompiledLocal {
                    name,
                    // The VM binds these when a call leaves the argument out.
                    default_value: defaults.get(index).cloned().flatten().map(|value| {
                        LiteralObject {
                            kind: LiteralObjectType::XString,
                            value: SourceRef::Shared(value.into()),
                            proc_body: None,
                        }
                    }),
                    frame_index: index as i32,
                    flag_mask: if name_is_temporary { VAR_TEMPORARY } else { 0 },
                })
                .collect::<Vec<_>>();
        let cmd_code_ranges = self
            .maps
            .iter()
            .map(|map| map.pc_range.start as i32..map.pc_range.end as i32)
            .collect::<Vec<_>>();
        let cmd_source_ranges = self
            .maps
            .iter()
            .map(|map| map.source_span.start as i32..map.source_span.end as i32)
            .collect::<Vec<_>>();
        let bytecode = ByteCode {
            header: None,
            info: ByteCodeInfo {
                num_commands: self.maps.len() as i32,
                num_src_bytes: self.source.len() as i32,
                num_code_bytes: self.code.len() as i32,
                num_lit_objects: self.literals.len() as i32,
                num_except_ranges: self.exceptions.len() as i32,
                num_aux_data_items: self.auxiliaries.len() as i32,
                num_cmd_loc_bytes: 0,
                max_except_depth: self
                    .exceptions
                    .iter()
                    .map(|e| e.nesting_level + 1)
                    .max()
                    .unwrap_or(0),
                max_stack_depth: 64,
                code_delta_size: 0,
                code_length_size: 0,
                src_delta_size: 0,
                src_length_size: 0,
            },
            code_bytes: self.code,
            cmd_code_ranges,
            cmd_source_ranges: Some(cmd_source_ranges),
            literals: self.literals,
            exceptions: self.exceptions,
            auxiliaries: self.auxiliaries,
            proc_num_args: argument_count as i32,
            proc_num_locals: locals.len() as i32,
            locals,
        };
        Ok(CompiledUnit {
            bytecode,
            maps: self.maps,
        })
    }

    fn compile_command(
        &mut self,
        span: Range<usize>,
        nesting_depth: u32,
    ) -> Result<(), CompileDiagnostic> {
        let text = &self.source[span.clone()];
        let words = scan_words(text)
            .map_err(|reason| diagnostic(self.path, self.source, span.clone(), text, reason))?;
        if words.is_empty() {
            return Err(diagnostic(
                self.path,
                self.source,
                span,
                text,
                "empty command".into(),
            ));
        }
        let anchor = self.code.len() as u32;
        // One-byte executable boundary owned by this source command. `Nop`
        // is intentionally used instead of Tcl's 9-byte StartCmd opcode so
        // the image map does not need to synthesize private compiler operands.
        self.emit(Opcode::Nop);
        let name = word_literal(text, words[0].clone()).unwrap_or_default();
        let deferred = runtime_command(
            text,
            &words,
            nesting_depth,
            self.static_declarations,
            self.proc_signature(&name).is_some(),
            self.procedure_context,
        );
        if deferred && self.mode == CompileMode::VmStrict && !vm_runtime_command(text, &words) {
            return Err(diagnostic(
                self.path,
                self.source,
                span,
                text,
                format!("{name:?} runs script through the Tcl host and requires hybrid mode"),
            ));
        }
        let builtin = (!deferred && self.proc_signature(&name).is_none())
            .then(|| name.strip_prefix("::").unwrap_or(&name));
        let string_form = (builtin == Some("string"))
            .then(|| strings::string_form(text, &words))
            .flatten();
        let dict_form = (builtin == Some("dict"))
            .then(|| dicts::dict_form(text, &words))
            .flatten();
        let clock = (builtin == Some("clock"))
            .then(|| clock_reading(text, &words))
            .flatten();
        let list_form = builtin.and_then(|name| lists::list_form(name, text, &words));
        let trace_inline = !deferred
            && (string_form.is_some()
                || dict_form.is_some()
                || clock.is_some()
                || list_form.is_some()
                || matches!(
                    name.as_str(),
                    "proc"
                        | "if"
                        | "while"
                        | "for"
                        | "foreach"
                        | "catch"
                        | "break"
                        | "continue"
                        | "expr"
                        | "return"
                )
                || builtin == Some("lmap"));
        let captured_start = self.captured_words.len();
        let mut trace_pc = 0;
        if trace_inline {
            for word in &words {
                self.compile_word(text, word.clone(), span.start, nesting_depth)?;
            }
            self.emit_arg(Opcode::List, words.len() as u32);
            trace_pc = self.code.len() as u32;
            self.emit(Opcode::Nop);
            self.captured_words.extend(
                words
                    .iter()
                    .enumerate()
                    .map(|(i, w)| (span.start + w.start..span.start + w.end, anchor, i)),
            );
        }
        if name == "proc" && !deferred {
            if !self.static_declarations || nesting_depth != 0 {
                return Err(diagnostic(
                    self.path,
                    self.source,
                    span,
                    text,
                    "nested/dynamic proc declarations are not admitted by V1".into(),
                ));
            }
            self.push_literal("");
            let (line, _) = line_column(self.source, span.start);
            let (end_line, _) = line_column(self.source, span.end.saturating_sub(1));
            let trace_end_pc = self.code.len() as u32;
            self.emit(Opcode::Nop);
            self.captured_words.truncate(captured_start);
            self.maps.push(VerifiedCommandMap {
                trace_pc,
                trace_end_pc,
                trace_argv_count: words.len(),
                trace_inline,
                trace_word_copies: self
                    .word_copies
                    .iter()
                    .filter(|(_, a, _)| *a == anchor)
                    .map(|(pc, _, i)| (*pc, *i))
                    .collect(),
                source_span: span,
                start_line: line,
                end_line,
                anchor_pc: anchor,
                pc_range: anchor..self.code.len() as u32,
                nesting_depth,
            });
            return Ok(());
        }
        let effect = command_effect(&name, text, &words, self.proc_signature(&name).is_some());
        if let CommandEffect::TclScriptOutsideImage(reason) = &effect {
            return Err(diagnostic(
                self.path,
                self.source,
                span,
                text,
                reason.clone(),
            ));
        }
        if self.mode == CompileMode::VmStrict
            && effect == CommandEffect::OpaqueWorldHost
            && is_tcl_builtin_command(&name)
        {
            return Err(diagnostic(
                self.path,
                self.source,
                span,
                text,
                format!("command {name:?} requires WorldHost and is not admitted by vm-strict"),
            ));
        }
        let expanded = words.iter().any(|w| expanded_word(&text[w.clone()]));
        if let Some(signature) = self.proc_signature(&name).filter(|_| !expanded) {
            let supplied = words.len() - 1;
            if !signature.accepts(supplied) {
                return Err(diagnostic(
                    self.path,
                    self.source,
                    span,
                    text,
                    format!(
                        "wrong # args for {name}: expected {}, got {supplied}",
                        signature.expected()
                    ),
                ));
            }
        }
        // Deferred commands, and native commands taking expanded words,
        // invoke the command with its final argument list.
        if deferred || expanded {
            if expanded {
                self.emit(Opcode::ExpandStart);
            }
            for word in &words {
                let expand = expanded_word(&text[word.clone()]);
                let start = word.start + if expand { 3 } else { 0 };
                self.compile_word(text, start..word.end, span.start, nesting_depth)?;
                if expand {
                    self.emit_arg(Opcode::ExpandStktop, words.len() as u32);
                }
            }
            trace_pc = self.code.len() as u32;
            self.emit(Opcode::Nop);
            if expanded {
                self.emit(Opcode::InvokeExpanded);
            } else {
                self.emit_arg(Opcode::InvokeStk1, words.len() as u32);
            }
        } else if matches!(name.as_str(), "if" | "while" | "for" | "foreach" | "catch")
            || builtin == Some("lmap")
        {
            self.compile_control(
                if builtin == Some("lmap") {
                    "lmap"
                } else {
                    &name
                },
                text,
                &words,
                span.clone(),
                nesting_depth,
            )?;
        } else if matches!(name.as_str(), "break" | "continue") {
            if words.len() != 1 {
                return Err(diagnostic(
                    self.path,
                    self.source,
                    span,
                    text,
                    "break/continue take no arguments".into(),
                ));
            }
            self.emit(if name == "break" {
                Opcode::Break
            } else {
                Opcode::Continue
            });
        } else if let Some(form) = &string_form {
            self.compile_string_form(form, text, &words, span.start, nesting_depth)?;
        } else if let Some(form) = &dict_form {
            self.compile_dict_form(form, text, &words, span.start, nesting_depth)?;
        } else if let Some(selector) = clock {
            self.emit_arg(Opcode::ClockRead, selector);
        } else if let Some(form) = list_form {
            self.compile_list_form(form, text, &words, span.start, nesting_depth)?;
        } else if name == "expr" {
            self.compile_expression_command(text, &words, span.clone(), nesting_depth)?;
            self.emit(Opcode::Nop);
        } else if name == "return"
            && self.return_context.last() == Some(&ExceptionType::Loop)
            && let Some(opcode) = control::static_loop_return(text, &words)
        {
            // Tcl's CompileReturnInternal discards static return options when
            // it can branch straight to an enclosing loop's break/continue.
            if words.len().is_multiple_of(2) {
                self.compile_word(
                    text,
                    words.last().unwrap().clone(),
                    span.start,
                    nesting_depth,
                )?;
                self.emit(Opcode::Pop);
            }
            self.emit(opcode);
        } else if name == "return" {
            if words.len() == 4
                && word_literal(text, words[1].clone()).ok().as_deref() == Some("-options")
            {
                // Tcl's dedicated return-options form feeds the dictionary
                // directly to returnStk, preserving its dictionary diagnostics.
                self.compile_word(text, words[2].clone(), span.start, nesting_depth)?;
                self.compile_word(text, words[3].clone(), span.start, nesting_depth)?;
            } else {
                // Tcl uses word-count parity, rather than an option whitelist or
                // `--`, to separate option pairs from the optional final result.
                let explicit_result = words.len().is_multiple_of(2);
                let option_end = words.len() - usize::from(explicit_result);
                for word in &words[1..option_end] {
                    self.compile_word(text, word.clone(), span.start, nesting_depth)?;
                }
                self.emit_arg(Opcode::List, (option_end - 1) as u32);
                if explicit_result {
                    self.compile_word(
                        text,
                        words.last().unwrap().clone(),
                        span.start,
                        nesting_depth,
                    )?;
                } else {
                    self.push_literal("");
                }
            }
            self.emit(Opcode::ReturnStk);
        } else {
            for word in &words {
                self.compile_word(text, word.clone(), span.start, nesting_depth)?;
            }
            trace_pc = self.code.len() as u32;
            self.emit(Opcode::Nop);
            self.emit_arg(Opcode::InvokeStk1, words.len() as u32);
        }
        let trace_end_pc = self.code.len() as u32;
        self.emit(Opcode::Nop);
        self.captured_words.truncate(captured_start);
        let (line, _) = line_column(self.source, span.start);
        let (end_line, _) = line_column(self.source, span.end.saturating_sub(1));
        self.maps.push(VerifiedCommandMap {
            trace_pc,
            trace_end_pc,
            trace_argv_count: words.len(),
            trace_inline,
            trace_word_copies: self
                .word_copies
                .iter()
                .filter(|(_, a, _)| *a == anchor)
                .map(|(pc, _, i)| (*pc, *i))
                .collect(),
            source_span: span,
            start_line: line,
            end_line,
            anchor_pc: anchor,
            pc_range: anchor..self.code.len() as u32,
            nesting_depth,
        });
        Ok(())
    }

    fn exception_level(&self, depth: u32) -> i32 {
        (depth + self.exception_bias) as i32
    }

    fn proc_signature(&self, name: &str) -> Option<ProcSignature> {
        self.proc_signatures
            .get(name)
            .copied()
            .or_else(|| self.proc_signatures.get(&format!("::{name}")).copied())
    }

    fn compile_word(
        &mut self,
        command: &str,
        word: Range<usize>,
        command_start: usize,
        nesting_depth: u32,
    ) -> Result<(), CompileDiagnostic> {
        let absolute = command_start + word.start..command_start + word.end;
        if let Some((_, anchor, index)) = self
            .captured_words
            .iter()
            .rev()
            .find(|(span, _, _)| *span == absolute)
            .cloned()
        {
            let pc = self.code.len() as u32;
            self.emit(Opcode::Nop);
            self.word_copies.push((pc, anchor, index));
            return Ok(());
        }
        let raw = &command[word.clone()];
        if raw.starts_with('{') {
            let value = braced_literal(command, word).map_err(|reason| {
                diagnostic(
                    self.path,
                    self.source,
                    command_start..command_start + command.len(),
                    command,
                    reason,
                )
            })?;
            self.push_literal(&value);
            return Ok(());
        }
        let (inner, inner_base) = if raw.starts_with('"') {
            (&raw[1..raw.len() - 1], command_start + word.start + 1)
        } else {
            (raw, command_start + word.start)
        };
        let mut pieces = 0u32;
        let bytes = inner.as_bytes();
        let mut cursor = 0usize;
        let mut literal_start = 0usize;
        while cursor < bytes.len() {
            match bytes[cursor] {
                b'\\' => {
                    if literal_start < cursor {
                        self.push_literal(&inner[literal_start..cursor]);
                        pieces += 1;
                    }
                    let (value, end) = backslash(inner, cursor).map_err(|reason| {
                        diagnostic(
                            self.path,
                            self.source,
                            inner_base + cursor..inner_base + cursor + 1,
                            command,
                            reason,
                        )
                    })?;
                    self.push_literal(&value);
                    pieces += 1;
                    cursor = end;
                    literal_start = cursor;
                }
                b'$' => {
                    let reference = words::variable(inner, cursor).map_err(|reason| {
                        diagnostic(
                            self.path,
                            self.source,
                            inner_base + cursor..inner_base + inner.len(),
                            command,
                            reason,
                        )
                    })?;
                    // A `$` that starts no variable name is an ordinary character.
                    let Some(reference) = reference else {
                        cursor += 1;
                        continue;
                    };
                    if literal_start < cursor {
                        self.push_literal(&inner[literal_start..cursor]);
                        pieces += 1;
                    }
                    self.push_literal(&inner[reference.name.clone()]);
                    match reference.index {
                        Some(index) => {
                            let offset = inner_base - command_start;
                            self.compile_word(
                                command,
                                offset + index.start..offset + index.end,
                                command_start,
                                nesting_depth,
                            )?;
                            self.emit(Opcode::LoadArrayStk);
                        }
                        None => self.emit(Opcode::LoadStk),
                    }
                    cursor = reference.end;
                    pieces += 1;
                    literal_start = cursor;
                }
                b'[' => {
                    if literal_start < cursor {
                        self.push_literal(&inner[literal_start..cursor]);
                        pieces += 1;
                    }
                    let close = matching_bracket(inner, cursor).map_err(|reason| {
                        diagnostic(
                            self.path,
                            self.source,
                            inner_base + cursor..inner_base + inner.len(),
                            command,
                            reason,
                        )
                    })?;
                    let nested_span = inner_base + cursor + 1..inner_base + close;
                    self.compile_script(nested_span, nesting_depth + 1)?;
                    pieces += 1;
                    cursor = close + 1;
                    literal_start = cursor;
                }
                _ => cursor += 1,
            }
        }
        if literal_start < inner.len() {
            self.push_literal(&inner[literal_start..]);
            pieces += 1;
        }
        if pieces == 0 {
            self.push_literal("");
        } else if pieces > 1 {
            self.emit_arg(Opcode::StrConcat1, pieces);
        }
        Ok(())
    }

    fn push_literal(&mut self, value: &str) {
        let index = self.literals.len();
        self.literals.push(LiteralObject {
            kind: LiteralObjectType::XString,
            value: SourceRef::Shared(value.into()),
            proc_body: None,
        });
        if index <= u8::MAX as usize {
            self.emit_arg(Opcode::Push1, index as u32);
        } else {
            self.emit_arg(Opcode::Push4, index as u32);
        }
    }
    fn emit(&mut self, opcode: Opcode) {
        self.code.push(opcode_byte(opcode));
    }
    fn emit_arg(&mut self, opcode: Opcode, arg: u32) {
        self.emit_args(opcode, &[arg]);
    }
    fn emit_args(&mut self, opcode: Opcode, args: &[u32]) {
        self.emit(opcode);
        let operands = TCL_86_OPCODE_TABLE
            .iter()
            .position(|candidate| *candidate == opcode)
            .and_then(|index| tcld_core::inst_table::TCL_86_INSTRUCTION_TABLE.get(index))
            .map_or(&[][..], |desc| desc.operands);
        for (index, arg) in args.iter().enumerate() {
            if operands.get(index).map(|op| op.mem_size()) == Some(1) {
                self.code.push(*arg as u8);
            } else {
                self.code.extend_from_slice(&arg.to_be_bytes());
            }
        }
    }
}

fn opcode_byte(opcode: Opcode) -> u8 {
    TCL_86_OPCODE_TABLE
        .iter()
        .position(|candidate| *candidate == opcode)
        .expect("Tcl 8.6 opcode") as u8
}

/// Parse a proc's argument list the way Tcl does: a list whose elements are
/// a name or a `{name default}` pair. Defaults stay literal text.
fn parse_proc_args(spec: &str) -> Result<Vec<ProcArg>, String> {
    let args = list_elements(spec)?
        .into_iter()
        .map(|specifier| {
            let mut fields = list_elements(&specifier)?.into_iter();
            let name = fields.next().ok_or("argument with no name")?;
            let default = fields.next();
            if fields.next().is_some() {
                return Err(format!(
                    "too many fields in argument specifier {specifier:?}"
                ));
            }
            if !valid_var_name(&name) {
                return Err(format!("V1 proc argument {name:?} is not a simple scalar"));
            }
            Ok(ProcArg { name, default })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut unique = std::collections::BTreeSet::new();
    for (index, arg) in args.iter().enumerate() {
        if !unique.insert(arg.name.as_str()) {
            return Err(format!("duplicate proc argument {:?}", arg.name));
        }
        if arg.name == "args" && index + 1 != args.len() {
            return Err("variadic `args` must be the final proc argument".into());
        }
    }
    Ok(args)
}

/// Elements of a Tcl list. List syntax is not script syntax: `$` and `[` are
/// ordinary characters, braced elements stay verbatim, and backslashes are
/// resolved only outside braces.
fn list_elements(text: &str) -> Result<Vec<String>, String> {
    let bytes = text.as_bytes();
    let push_char = |element: &mut String, i: &mut usize| -> Result<(), String> {
        if bytes[*i] == b'\\' {
            let (value, end) = backslash(text, *i)?;
            element.push_str(&value);
            *i = end;
        } else {
            let ch = text[*i..].chars().next().expect("in bounds");
            element.push(ch);
            *i += ch.len_utf8();
        }
        Ok(())
    };
    let mut elements = Vec::new();
    let mut i = 0;
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i == bytes.len() {
            return Ok(elements);
        }
        let mut element = String::new();
        match bytes[i] {
            b'{' => {
                let start = i + 1;
                let mut depth = 1;
                while depth > 0 {
                    i += 1;
                    match bytes.get(i) {
                        None => return Err("unmatched open brace in list".into()),
                        Some(b'\\') => i += 1,
                        Some(b'{') => depth += 1,
                        Some(b'}') => depth -= 1,
                        Some(_) => {}
                    }
                }
                element.push_str(&text[start..i]);
                i += 1;
            }
            b'"' => {
                i += 1;
                loop {
                    match bytes.get(i) {
                        None => return Err("unmatched open quote in list".into()),
                        Some(b'"') => break,
                        Some(_) => push_char(&mut element, &mut i)?,
                    }
                }
                i += 1;
            }
            _ => {
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                    push_char(&mut element, &mut i)?;
                }
            }
        }
        if i < bytes.len() && !bytes[i].is_ascii_whitespace() {
            return Err("list element in braces or quotes followed by extra characters".into());
        }
        elements.push(element);
    }
}

use words::scan_words;

fn expanded_word(raw: &str) -> bool {
    raw.starts_with("{*}")
        && raw
            .as_bytes()
            .get(3)
            .is_some_and(|b| !b.is_ascii_whitespace())
}

fn word_literal(text: &str, span: Range<usize>) -> Result<String, String> {
    let raw = &text[span];
    if expanded_word(raw) {
        return Err("expanded word is evaluated at runtime".into());
    }
    if raw.starts_with('{') {
        braced_literal_raw(raw)
    } else {
        let inner = if raw.starts_with('"') {
            if raw.len() < 2 || !raw.ends_with('"') {
                return Err("unterminated quoted word".into());
            }
            &raw[1..raw.len() - 1]
        } else {
            raw
        };
        let mut result = String::new();
        let mut i = 0;
        while i < inner.len() {
            match inner.as_bytes()[i] {
                b'$' | b'[' => return Err("word must be static".into()),
                b'\\' => {
                    let (value, end) = backslash(inner, i)?;
                    result.push_str(&value);
                    i = end;
                }
                _ => {
                    let ch = inner[i..].chars().next().unwrap();
                    result.push(ch);
                    i += ch.len_utf8();
                }
            }
        }
        Ok(result)
    }
}

fn braced_literal(text: &str, span: Range<usize>) -> Result<String, String> {
    braced_literal_raw(&text[span])
}
/// The value of a braced word: its text, except that a backslash-newline and
/// the spaces and tabs after it read as one space, as `Tcl_ParseBraces`
/// substitutes them. Every other backslash sequence stays as written.
fn braced_literal_raw(raw: &str) -> Result<String, String> {
    if raw.len() < 2 || !raw.starts_with('{') || !raw.ends_with('}') {
        return Err("expected braced literal".into());
    }
    let inner = &raw[1..raw.len() - 1];
    let bytes = inner.as_bytes();
    let mut value = String::with_capacity(inner.len());
    let (mut i, mut literal_start) = (0, 0);
    while i < bytes.len() {
        if bytes[i] != b'\\' {
            i += 1;
        } else if bytes.get(i + 1) == Some(&b'\n') {
            value.push_str(&inner[literal_start..i]);
            i += 2;
            while matches!(bytes.get(i), Some(b' ' | b'\t')) {
                i += 1;
            }
            value.push(' ');
            literal_start = i;
        } else {
            i += 2;
        }
    }
    value.push_str(&inner[literal_start.min(inner.len())..]);
    Ok(value)
}
/// A bare word whose value is its own text, such as the `break` in
/// `if {$done} break`: Tcl compiles it as a script in place.
fn bare_script_word(text: &str, word: &Range<usize>) -> bool {
    let raw = &text[word.clone()];
    !raw.starts_with(['{', '"']) && word_literal(text, word.clone()).is_ok_and(|value| value == raw)
}

fn valid_var_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b':'))
}
/// The offset of the `]` closing the command substitution that opens at
/// `open`.
fn matching_bracket(text: &str, open: usize) -> Result<usize, String> {
    words::substitution_end(text.as_bytes(), open)
}
fn line_column(source: &str, offset: usize) -> (u32, u32) {
    // End-line lookups use end_byte - 1, which may fall inside the final
    // UTF-8 character. Source maps remain byte based; columns count characters.
    let mut offset = offset.min(source.len());
    while !source.is_char_boundary(offset) {
        offset -= 1;
    }
    let prefix = &source[..offset];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() as u32 + 1;
    let column = prefix
        .rsplit_once('\n')
        .map_or(prefix.len(), |(_, tail)| tail.len()) as u32
        + 1;
    (line, column)
}
fn diagnostic(
    path: &Path,
    source: &str,
    span: Range<usize>,
    command: &str,
    reason: String,
) -> CompileDiagnostic {
    let (line, column) = line_column(source, span.start);
    CompileDiagnostic {
        path: path.to_path_buf(),
        span,
        line,
        column,
        command: command.trim().to_string(),
        reason,
    }
}
fn hex_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn image_identity_digest(
    schema: &str,
    compiler_semantics: &str,
    mode: CompileMode,
    source: &str,
    procedure_context: bool,
) -> String {
    let mut identity = Sha256::new();
    identity.update(schema);
    identity.update([0]);
    identity.update(compiler_semantics);
    identity.update([0]);
    identity.update(match mode {
        CompileMode::Hybrid => b"hybrid".as_slice(),
        CompileMode::VmStrict => b"vm-strict".as_slice(),
    });
    identity.update([0]);
    identity.update([u8::from(procedure_context)]);
    identity.update(source.as_bytes());
    format!("{:x}", identity.finalize())
}

/// Runtime-controlled forms are admitted as mapped invocations. Their evaluated
/// argument values are compiled by the VM before any generated body executes.
/// Whether vm-strict can run a runtime command itself: the VM compiles its
/// generated source under vm-strict, and resolves a computed command name when
/// it runs, where host commands stay denied. Tcl implements the excluded
/// commands with its event loop, file system, or callbacks into script.
fn vm_runtime_command(text: &str, words: &[Range<usize>]) -> bool {
    let Some(name) = words
        .first()
        .and_then(|word| word_literal(text, word.clone()).ok())
    else {
        return true;
    };
    !matches!(
        name.strip_prefix("::").unwrap_or(&name),
        "source"
            | "after"
            | "update"
            | "vwait"
            | "trace"
            | "fileevent"
            | "chan"
            | "subst"
            | "time"
            | "tailcall"
    ) && dict_script(text, words) != Some(true)
}

/// Whether a `dict` command evaluates a script: `for`, `map`, `update` and
/// `with`, and `filter` with the `script` filter type. `None` when its words
/// do not say, because the subcommand or filter type is computed.
fn dict_script(text: &str, words: &[Range<usize>]) -> Option<bool> {
    let literal = |index: usize| match words.get(index) {
        Some(word) if expanded_word(&text[word.clone()]) => None,
        Some(word) => word_literal(text, word.clone()).ok().map(Some),
        None => Some(None),
    };
    let name = literal(0)??;
    if name.strip_prefix("::").unwrap_or(&name) != "dict" {
        return Some(false);
    }
    Some(match literal(1)?.as_deref().and_then(dict_subcommand) {
        Some("for" | "map" | "update" | "with") => true,
        Some("filter") => {
            literal(3)?.and_then(|filter| unique_prefix(&["key", "script", "value"], &filter))
                == Some("script")
        }
        _ => false,
    })
}

fn runtime_command(
    text: &str,
    words: &[Range<usize>],
    depth: u32,
    static_declarations: bool,
    static_proc: bool,
    procedure_context: bool,
) -> bool {
    let name = words
        .first()
        .and_then(|w| word_literal(text, w.clone()).ok())
        .unwrap_or_default();
    if words.iter().any(|w| expanded_word(&text[w.clone()])) {
        // A native command takes expanded words like any other arguments.
        return !lists::is_native_command(&name);
    }
    match name.trim_start_matches("::") {
        "" | "eval" | "source" | "uplevel" | "apply" | "try" | "switch" | "subst" | "time"
        | "tailcall" | "after" | "update" | "vwait" | "trace" | "fileevent" => true,
        "dict" => {
            dicts::dict_form(text, words).is_none() && dict_script(text, words) != Some(false)
        }
        "chan" => words
            .get(1)
            .is_none_or(|w| word_literal(text, w.clone()).map_or(true, |s| s == "event")),
        "namespace" => words.get(1).is_none_or(|w| {
            word_literal(text, w.clone())
                .map_or(true, |s| matches!(s.as_str(), "eval" | "inscope" | "code"))
        }),
        "proc" => {
            !static_declarations || depth != 0 || parse_proc_decl(text, 0..text.len()).is_err()
        }
        "for" | "while" | "if" | "catch" => words[1..]
            .iter()
            .any(|word| word_literal(text, word.clone()).is_err()),
        "foreach" => {
            words
                .last()
                .is_some_and(|word| word_literal(text, word.clone()).is_err())
                || words
                    .get(1..words.len().saturating_sub(1))
                    .unwrap_or(&[])
                    .iter()
                    .step_by(2)
                    .any(|word| word_literal(text, word.clone()).is_err())
        }
        "lmap" => !procedure_context || !control::inline_lmap(text, words),
        "expr" => words.get(1).is_some_and(|word| {
            !text[word.clone()].starts_with('{')
                && text[word.start..words.last().unwrap().end].contains(['$', '['])
        }),
        _ => {
            let _ = static_proc;
            false
        }
    }
}

fn backslash(text: &str, start: usize) -> Result<(String, usize), String> {
    let mut i = start + 1;
    let Some(ch) = text[i..].chars().next() else {
        return Ok(("\\".into(), i));
    };
    i += ch.len_utf8();
    let simple = match ch {
        'a' => Some('\u{7}'),
        'b' => Some('\u{8}'),
        'f' => Some('\u{c}'),
        'n' => Some('\n'),
        'r' => Some('\r'),
        't' => Some('\t'),
        'v' => Some('\u{b}'),
        _ => None,
    };
    if let Some(value) = simple {
        return Ok((value.to_string(), i));
    }
    if ch == '\n' {
        while text
            .as_bytes()
            .get(i)
            .is_some_and(|b| matches!(b, b' ' | b'\t'))
        {
            i += 1;
        }
        return Ok((" ".into(), i));
    }
    let (radix, max) = match ch {
        'x' => (16, 2),
        'u' => (16, 4),
        'U' => (16, 8),
        '0'..='7' => (8, if ch <= '3' { 3 } else { 2 }),
        _ => return Ok((ch.to_string(), i)),
    };
    let begin = if radix == 8 { start + 1 } else { i };
    let mut end = begin;
    while end < text.len()
        && end - begin < max
        && text.as_bytes()[end].is_ascii()
        && (text.as_bytes()[end] as char).is_digit(radix)
    {
        end += 1;
    }
    if end == begin {
        return Ok((ch.to_string(), i));
    }
    let value = u32::from_str_radix(&text[begin..end], radix).map_err(|e| e.to_string())?;
    Ok((char::from_u32(value).unwrap_or('\u{fffd}').to_string(), end))
}

/// Lossless Tcl word representation used to capture already-evaluated argument
/// values. Prefer braces so ordinary script bodies retain readable source maps.
pub fn quote_word(value: &str) -> String {
    let mut depth = 0i64;
    let mut escaped = false;
    let mut braced = true;
    for ch in value.chars() {
        if escaped {
            if ch == '\n' {
                braced = false;
            }
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth < 0 {
                    braced = false;
                }
            }
            _ => {}
        }
    }
    if braced && depth == 0 && !escaped {
        return format!("{{{value}}}");
    }
    let mut result = String::from("\"");
    for ch in value.chars() {
        if matches!(ch, '\\' | '"' | '$' | '[' | ']') {
            result.push('\\');
        }
        result.push(ch);
    }
    result.push('"');
    result
}

/// Apply the compiler's script-effect boundary to a command whose name or
/// arguments became known at runtime. Unsupported evaluators never slip into
/// the opaque Tcl leaf bridge through a computed command name/subcommand.
pub fn validate_runtime_invocation(values: &[String], mode: CompileMode) -> Result<(), String> {
    let name = values.first().ok_or("empty invocation")?;
    let source = values
        .iter()
        .map(|word| quote_word(word))
        .collect::<Vec<_>>()
        .join(" ");
    let words = scan_words(&source)?;
    match command_effect(name, &source, &words, false) {
        CommandEffect::TclScriptOutsideImage(reason) => Err(reason),
        CommandEffect::OpaqueWorldHost if mode == CompileMode::VmStrict => {
            Err("WorldHost invocation requires hybrid mode".into())
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifier_checks_execution_arguments_and_boundaries() {
        let image = compile_source(
            "/trace.tcl".into(),
            "foreach x {1 2} {incr x}; return $x".into(),
            CompileMode::Hybrid,
        )
        .unwrap();
        let mut changed = image.clone();
        changed.units[changed.top_level_unit].commands[0].trace_pc = 0;
        assert!(changed.verify().is_err());
        let mut changed = image.clone();
        changed.units[changed.top_level_unit].commands[0].trace_argv_count += 1;
        assert!(changed.verify().is_err());
        let mut changed = image.clone();
        let map = changed.units[changed.top_level_unit]
            .commands
            .iter_mut()
            .find(|m| !m.trace_word_copies.is_empty())
            .unwrap();
        map.trace_word_copies[0].1 = map.trace_argv_count;
        assert!(changed.verify().is_err());
    }

    #[test]
    fn eof_terminates_commands_without_changing_source_identity() {
        for source in [
            "set x 73",
            "set x héllo\nputs $x",
            "proc inc {x} {return [expr {$x + 1}]}\nset x [inc 2]",
        ] {
            let image = compile_source(
                PathBuf::from("/tmp/eof.tcl"),
                source.into(),
                CompileMode::Hybrid,
            )
            .unwrap();
            image.verify().unwrap();
            assert_eq!(image.source.content.as_ref(), source);
            assert_eq!(image.source.content_sha256, hex_digest(source.as_bytes()));
            assert!(
                image
                    .units
                    .iter()
                    .flat_map(|unit| &unit.commands)
                    .all(|map| map.source_span.end <= source.len())
            );
        }
        assert!(
            compile_source(
                PathBuf::from("/tmp/eof.tcl"),
                "set x {unclosed".into(),
                CompileMode::Hybrid
            )
            .is_err()
        );
    }

    #[test]
    fn deterministic_image_maps_nested_commands_and_procs() {
        let source = "proc inc {x} {\n  return [expr {$x + 1}]\n}\nset n 2\nset n [inc $n]\n";
        let a = compile_source(
            PathBuf::from("/tmp/demo.tcl"),
            source.into(),
            CompileMode::VmStrict,
        )
        .unwrap();
        let b = compile_source(
            PathBuf::from("/tmp/demo.tcl"),
            source.into(),
            CompileMode::VmStrict,
        )
        .unwrap();
        assert_eq!(a.image_digest, b.image_digest);
        assert_eq!(a.source.content_sha256, b.source.content_sha256);
        assert_eq!(a.units.len(), 2);
        assert!(
            a.units
                .iter()
                .flat_map(|unit| &unit.commands)
                .any(|map| map.nesting_depth == 1)
        );
        let mut parsed = source::command_spans(source, false)
            .unwrap()
            .into_iter()
            .map(|span| (span.start_byte, span.end_byte))
            .collect::<Vec<_>>();
        let mut mapped = a
            .units
            .iter()
            .flat_map(|unit| &unit.commands)
            .map(|map| (map.source_span.start, map.source_span.end))
            .collect::<Vec<_>>();
        parsed.sort_unstable();
        mapped.sort_unstable();
        assert_eq!(
            mapped, parsed,
            "every parsed command is mapped exactly once"
        );
        for unit in &a.units {
            for nested in unit.commands.iter().filter(|map| map.nesting_depth > 0) {
                let parent = unit
                    .commands
                    .iter()
                    .filter(|candidate| candidate.nesting_depth < nested.nesting_depth)
                    .filter(|candidate| {
                        candidate.source_span.start <= nested.source_span.start
                            && candidate.source_span.end >= nested.source_span.end
                    })
                    .max_by_key(|candidate| candidate.nesting_depth)
                    .expect("nested command has a source parent");
                assert!(parent.pc_range.start < nested.pc_range.start);
                assert!(parent.pc_range.end > nested.pc_range.end);
            }
            if !unit.commands.is_empty() {
                for instruction in unit.bytecode.get_instructions_raw().unwrap() {
                    let pc = instruction.location as u32;
                    let deepest = unit
                        .commands
                        .iter()
                        .filter(|map| map.pc_range.contains(&pc))
                        .max_by_key(|map| map.nesting_depth);
                    assert!(deepest.is_some(), "instruction PC {pc} has a source owner");
                }
            }
        }
        a.verify().unwrap();

        let mut duplicated = a.clone();
        let duplicate = duplicated.top_level().commands[0].clone();
        duplicated.units[duplicated.top_level_unit]
            .commands
            .push(duplicate);
        assert!(duplicated.verify().is_err());
    }

    #[test]
    fn vm_strict_rejects_world_host_with_precise_fallback() {
        let error = compile_source(
            PathBuf::from("/tmp/demo.tcl"),
            "exec ls\n".into(),
            CompileMode::VmStrict,
        )
        .unwrap_err();
        let rendered = error.to_string();
        assert!(rendered.contains("/tmp/demo.tcl:1:1"));
        assert!(rendered.contains("WorldHost"));
        assert!(rendered.contains("--backend=host"));
    }

    #[test]
    fn lmap_local_compilation_preserves_maps_and_runtime_fallbacks() {
        for mode in [CompileMode::Hybrid, CompileMode::VmStrict] {
            let source = "lmap x {1 2} {incr x}";
            let global = compile_source("/workspace/lmap.tcl".into(), source.into(), mode).unwrap();
            let local =
                compile_source_in_context("/workspace/lmap.tcl".into(), source.into(), mode, true)
                    .unwrap();
            assert_ne!(global.image_digest, local.image_digest);
            for (image, collects) in [(&global, false), (&local, true)] {
                image.verify().unwrap();
                assert_eq!(
                    image
                        .top_level()
                        .bytecode
                        .get_instructions_raw()
                        .unwrap()
                        .iter()
                        .any(|inst| inst.opcode == Opcode::LmapCollect),
                    collects
                );
                assert_eq!(
                    image
                        .top_level()
                        .commands
                        .iter()
                        .any(|map| &image.source.content[map.source_span.clone()] == "incr x"),
                    collects
                );
            }
            for source in [
                "lmap a(k) {1} {set a(k)}",
                "lmap $vars {1} $body",
                "lmap {} {1} {error wrong}",
            ] {
                let image = compile_source_in_context(
                    "/workspace/lmap.tcl".into(),
                    source.into(),
                    mode,
                    true,
                )
                .unwrap();
                assert!(
                    !image
                        .top_level()
                        .bytecode
                        .get_instructions_raw()
                        .unwrap()
                        .iter()
                        .any(|inst| inst.opcode == Opcode::LmapCollect)
                );
            }
        }
    }

    #[test]
    fn hybrid_admits_opaque_leaf_calls_but_rejects_tcl_script_evaluators() {
        let leaf = compile_source(
            PathBuf::from("/tmp/leaf.tcl"),
            "puts hello\nerror boom\nvendor::leaf value\n".into(),
            CompileMode::Hybrid,
        )
        .unwrap();
        assert_eq!(leaf.top_level().commands.len(), 3);
        leaf.verify().unwrap();

        for (label, source) in [
            ("interp eval", "interp eval {} {set hidden 1}\n"),
            ("interp create", "interp create child\n"),
            (
                "interp invokehidden",
                "interp invokehidden child source hidden.tcl\n",
            ),
            ("package require", "package require hidden\n"),
            (
                "oo::class create",
                "oo::class create Hidden {method x {} {}}\n",
            ),
            ("oo::class new", "oo::class new {method x {} {}}\n"),
            ("oo::object create", "oo::object create hidden SomeClass\n"),
            ("oo::object new", "oo::object new SomeClass\n"),
        ] {
            let error = compile_source(
                PathBuf::from(format!("/tmp/{label}.tcl")),
                source.into(),
                CompileMode::Hybrid,
            )
            .unwrap_err();
            assert_eq!(error.line, 1, "{label}: {error}");
            assert!(
                error.reason.contains("outside the debug image"),
                "{label}: {error}"
            );
            assert!(error.to_string().contains("--backend=host"), "{label}");
        }
    }

    #[test]
    fn verifier_rejects_tampered_known_evaluators_even_with_valid_digests() {
        for (admitted, replacement, expected) in [
            (
                "package provide demo 1\n",
                "package require demo 1\n",
                "package require",
            ),
            ("oo::class foo\n", "oo::class new\n", "oo::class new"),
        ] {
            assert_eq!(admitted.len(), replacement.len());
            let mut image = compile_source(
                PathBuf::from("/tmp/tampered.tcl"),
                admitted.into(),
                CompileMode::Hybrid,
            )
            .unwrap();
            image.source.content = Arc::from(replacement);
            image.source.content_sha256 = hex_digest(replacement.as_bytes());
            image.image_digest = image_identity_digest(
                image.schema,
                image.compiler_semantics,
                image.mode,
                replacement,
                image.procedure_context,
            );

            let error = image.verify().unwrap_err();
            assert!(error.contains(expected), "{expected}: {error}");
            assert!(
                error.contains("outside the debug image"),
                "{expected}: {error}"
            );
        }
    }

    #[test]
    fn malformed_source_never_produces_an_image() {
        assert!(
            compile_source(
                PathBuf::from("/tmp/bad.tcl"),
                "set x {\n".into(),
                CompileMode::Hybrid
            )
            .is_err()
        );
    }

    #[test]
    fn utf8_command_end_is_a_valid_source_location() {
        let image = compile_source(
            "/workspace/unicode.tcl".into(),
            "list 世界\nset x λ".into(),
            CompileMode::Hybrid,
        )
        .unwrap();
        image.verify().unwrap();
        assert_eq!(image.units[0].commands.last().unwrap().end_line, 2);
    }

    #[test]
    fn schema_and_semantics_change_image_identity() {
        let image = compile_source(
            PathBuf::from("/tmp/a.tcl"),
            "set x 1\n".into(),
            CompileMode::Hybrid,
        )
        .unwrap();
        assert_eq!(image.schema, DEBUG_IMAGE_SCHEMA);
        assert_eq!(image.compiler_semantics, COMPILER_SEMANTICS);
        assert_ne!(image.image_digest, image.source.content_sha256);
        assert_ne!(
            image.image_digest,
            image_identity_digest(
                "tcld.debug-image/v2",
                COMPILER_SEMANTICS,
                image.mode,
                "set x 1\n",
                false
            )
        );
        assert_ne!(
            image.image_digest,
            image_identity_digest(
                DEBUG_IMAGE_SCHEMA,
                "different-policy",
                image.mode,
                "set x 1\n",
                false
            )
        );
        assert_ne!(
            image.image_digest,
            image_identity_digest(
                DEBUG_IMAGE_SCHEMA,
                COMPILER_SEMANTICS,
                CompileMode::VmStrict,
                "set x 1\n",
                false
            )
        );
        assert_ne!(
            image.image_digest,
            image_identity_digest(
                DEBUG_IMAGE_SCHEMA,
                COMPILER_SEMANTICS,
                image.mode,
                "set x 2\n",
                false
            )
        );

        let mut corrupt_source_digest = image.clone();
        corrupt_source_digest.source.content_sha256 = "0".repeat(64);
        assert!(corrupt_source_digest.verify().is_err());

        let mut corrupt_image_digest = image.clone();
        corrupt_image_digest.image_digest = "0".repeat(64);
        assert!(corrupt_image_digest.verify().is_err());
        let mut corrupt_context = image.clone();
        corrupt_context.procedure_context = true;
        assert!(corrupt_context.verify().is_err());
    }

    #[test]
    fn proc_declarations_and_calls_are_admitted_before_image_creation() {
        let wrong_arity = compile_source(
            PathBuf::from("/tmp/arity.tcl"),
            "proc add {a b} { expr {$a + $b} }\nadd 1\n".into(),
            CompileMode::VmStrict,
        )
        .unwrap_err();
        assert!(wrong_arity.reason.contains("wrong # args"), "{wrong_arity}");

        let compile = |source: &str| {
            compile_source(
                PathBuf::from("/tmp/default.tcl"),
                source.into(),
                CompileMode::VmStrict,
            )
        };
        let image = compile("proc greet {{name world}} { return $name }\ngreet\ngreet you\n")
            .expect("defaulted arguments are admitted");
        let greet = image.unit("proc:greet").expect("greet unit");
        let defaults = greet
            .bytecode
            .locals
            .iter()
            .map(|local| {
                local
                    .default_value
                    .as_ref()
                    .map(|value| value.value.source_repr())
            })
            .collect::<Vec<_>>();
        assert_eq!(defaults, [Some("world".to_string())]);

        // Tcl binds positionally: a later parameter without a default makes
        // the defaulted one before it required as well.
        for (source, admitted) in [
            ("proc f {a {b 2}} {}\nf 1\nf 1 2\n", true),
            ("proc f {a {b 2}} {}\nf\n", false),
            ("proc f {a {b 2}} {}\nf 1 2 3\n", false),
            ("proc f {{a 1} b} {}\nf 1\n", false),
            ("proc f {{a 1} b} {}\nf 1 2\n", true),
            ("proc f {{a 1} args} {}\nf\nf 1 2 3\n", true),
        ] {
            let result = compile(source);
            assert_eq!(result.is_ok(), admitted, "{source}: {result:?}");
            if let Err(error) = result {
                assert!(
                    error.reason.contains("wrong # args for f: expected"),
                    "{error}"
                );
            }
        }

        // An argument list is a Tcl list: defaults are literal text.
        for (spec, expected) in [
            ("{{v $x}}", "$x"),
            ("{{v [list]}}", "[list]"),
            ("{{v {}}}", ""),
            ("{{v {a {b c}}}}", "a {b c}"),
            ("{{v \"q r\"}}", "q r"),
        ] {
            let image = compile(&format!("proc d {spec} {{ return $v }}\n"))
                .unwrap_or_else(|error| panic!("{spec}: {error}"));
            let local = &image.unit("proc:d").unwrap().bytecode.locals[0];
            assert_eq!(
                local
                    .default_value
                    .as_ref()
                    .map(|value| value.value.source_repr()),
                Some(expected.to_string()),
                "{spec}"
            );
        }
        let fields = compile("proc d {{v 1 2}} {}\n").unwrap_err();
        assert!(fields.reason.contains("too many fields"), "{fields}");
        let unnamed = compile("proc d {{}} {}\n").unwrap_err();
        assert!(
            unnamed.reason.contains("argument with no name"),
            "{unnamed}"
        );

        let misplaced_args = compile_source(
            PathBuf::from("/tmp/args.tcl"),
            "proc gather {args tail} { return $tail }\n".into(),
            CompileMode::VmStrict,
        )
        .unwrap_err();
        assert!(
            misplaced_args.reason.contains("must be the final"),
            "{misplaced_args}"
        );
    }
}
