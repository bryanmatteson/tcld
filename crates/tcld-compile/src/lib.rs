//! Deterministic Tcl source -> in-process debugger image compilation.
//!
//! This crate is the sole owner of source admission and verified source/PC
//! maps. It never evaluates Tcl and never shells out to a Tcl compiler.
//! Unknown commands in Hybrid mode are admitted only as opaque WorldHost leaf
//! calls: the image maps their outer source command and makes no claim about
//! implementation work inside the external command. Known Tcl commands/forms
//! that evaluate scripts are rejected until the compiler can lower and map
//! those bodies itself.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sha2::{Digest, Sha256};
use tcld_core::bytecode::{
    ByteCode, ByteCodeInfo, CompiledLocal, LiteralObject, LiteralObjectType,
};
use tcld_core::inst_table::{Opcode, TCL_86_OPCODE_TABLE};
use tcld_core::source::SourceRef;

pub const DEBUG_IMAGE_SCHEMA: &str = "tcld.debug-image/v1";
pub const COMPILER_SEMANTICS: &str = "tcld.source-compiler/v1.1-tcl86";

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
                            "command PC overlap is not source-nested in {}",
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
        let spans = tcld_syntax::command_spans(&self.source.content)
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
                .and_then(|word| word_literal(text, word.clone()))?;
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
                CommandEffect::OpaqueWorldHost if self.mode == CompileMode::VmStrict => {
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
    let tree = tcld_syntax::parse_tcl(&source).map_err(|error| {
        diagnostic(
            &path,
            &source,
            0..source.len().min(1),
            "",
            error.to_string(),
        )
    })?;
    if tree.root_node().has_error() {
        let error = first_error_node(tree.root_node()).unwrap_or(tree.root_node());
        return Err(diagnostic(
            &path,
            &source,
            error.start_byte()
                ..error
                    .end_byte()
                    .max(error.start_byte() + 1)
                    .min(source.len()),
            source.get(error.byte_range()).unwrap_or(""),
            "Tcl syntax is incomplete or malformed".into(),
        ));
    }
    let spans = tcld_syntax::command_spans(&source).map_err(|error| {
        diagnostic(
            &path,
            &source,
            0..source.len().min(1),
            "",
            error.to_string(),
        )
    })?;
    let direct_top = direct_commands(&spans, 0..source.len());
    let mut saw_executable = false;
    for span in &direct_top {
        if span.name.as_deref() == Some("proc") {
            if saw_executable {
                return Err(diagnostic(
                    &path,
                    &source,
                    span.start_byte..span.end_byte,
                    &span.text,
                    "V1 static proc declarations must precede executable top-level commands".into(),
                ));
            }
        } else {
            saw_executable = true;
        }
    }
    let mut proc_decls = Vec::new();
    let mut proc_signatures = std::collections::BTreeMap::new();
    for span in &direct_top {
        if span.name.as_deref() != Some("proc") {
            continue;
        }
        let decl = parse_proc_decl(&source, span.start_byte..span.end_byte).map_err(|reason| {
            diagnostic(
                &path,
                &source,
                span.start_byte..span.end_byte,
                &span.text,
                reason,
            )
        })?;
        let signature = ProcSignature::from_args(&decl.args)?;
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

    let top = Compiler::new(&path, &source, mode, &proc_signatures, true).compile_unit(
        &direct_top,
        0..source.len(),
        None,
    )?;
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
        image_digest: image_identity_digest(DEBUG_IMAGE_SCHEMA, COMPILER_SEMANTICS, mode, &source),
        source: DebugSourceIdentity {
            path,
            content: Arc::from(source),
            content_sha256,
        },
        abi: DebugTclAbi::Tcl86,
        mode,
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

fn first_error_node(mut node: tree_sitter::Node<'_>) -> Option<tree_sitter::Node<'_>> {
    loop {
        if node.is_error() || node.is_missing() {
            return Some(node);
        }
        let mut cursor = node.walk();
        let child = node.children(&mut cursor).find(|child| child.has_error())?;
        node = child;
    }
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
    args: Vec<String>,
    body_span: Range<usize>,
}

#[derive(Clone, Copy)]
struct ProcSignature {
    fixed: usize,
    variadic: bool,
}
impl ProcSignature {
    fn from_args(args: &[String]) -> Result<Self, CompileDiagnostic> {
        Ok(Self {
            fixed: args.len() - usize::from(args.last().is_some_and(|name| name == "args")),
            variadic: args.last().is_some_and(|name| name == "args"),
        })
    }
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
    let args = parse_simple_args(&args_spec)?;
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
    let name = name.strip_prefix("::").unwrap_or(name);
    if matches!(
        name,
        "set" | "return" | "expr" | "append" | "lappend" | "incr" | "unset" | "global" | "variable"
    ) || is_static_proc
    {
        return CommandEffect::CompilerOwned;
    }
    let rejects_script = |form: &str| {
        CommandEffect::TclScriptOutsideImage(format!(
            "{form} evaluates or schedules Tcl script outside the debug image; V1 cannot guarantee canonical stepping"
        ))
    };
    let rejects_control = |form: &str| {
        CommandEffect::TclScriptOutsideImage(format!(
            "{form} performs control flow outside the debug image; V1 cannot guarantee canonical stepping"
        ))
    };
    match name {
        "source" => CommandEffect::TclScriptOutsideImage(
            "source loads Tcl script outside the debug image; V1 cannot guarantee canonical stepping"
                .into(),
        ),
        "proc" => CommandEffect::TclScriptOutsideImage(
            "proc declaration has no compiler-owned activation unit; V1 cannot guarantee canonical stepping"
                .into(),
        ),
        "eval" | "uplevel" | "apply" | "if" | "while" | "for" | "foreach" | "lmap"
        | "switch" | "catch" | "try" | "time" | "subst" => rejects_script(name),
        "tailcall" | "coroutine" | "break" | "continue" | "yield" | "yieldto" => {
            rejects_control(name)
        }
        "exit" => CommandEffect::TclScriptOutsideImage(
            "exit would terminate the debugger host outside the debug image; V1 does not admit it"
                .into(),
        ),
        "namespace" => match static_subcommand(text, words) {
            Some(subcommand)
                if matches!(
                    subcommand.as_str(),
                    "eval" | "inscope" | "code" | "unknown" | "ensemble"
                ) =>
            {
                rejects_script(&format!("namespace {subcommand}"))
            }
            Some(_) => CommandEffect::OpaqueWorldHost,
            None => rejects_script("namespace with a dynamic subcommand"),
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
        "dict" => match static_subcommand(text, words) {
            Some(subcommand)
                if matches!(
                    subcommand.as_str(),
                    "for" | "map" | "update" | "with" | "filter"
                ) =>
            {
                rejects_script(&format!("dict {subcommand}"))
            }
            Some(_) => CommandEffect::OpaqueWorldHost,
            None => rejects_script("dict with a dynamic subcommand"),
        },
        "after" => match static_subcommand(text, words) {
            Some(subcommand)
                if subcommand == "idle"
                    || (subcommand.parse::<u64>().is_ok() && words.len() > 2) =>
            {
                rejects_script("after callback")
            }
            Some(_) => CommandEffect::OpaqueWorldHost,
            None => rejects_script("after with a dynamic callback form"),
        },
        "trace" | "fileevent" | "vwait" | "update" | "bind" | "oo::define"
        | "oo::objdefine" | "unknown" | "auto_load" | "auto_import" | "timerate"
        | "history" => rejects_script(name),
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
            if words.iter().skip(1).any(|word| {
                word_literal(text, word.clone())
                    .ok()
                    .as_deref()
                    == Some("-server")
            }) {
                rejects_script("socket -server")
            } else {
                CommandEffect::OpaqueWorldHost
            }
        }
        "lsort" => {
            if words.iter().skip(1).any(|word| {
                word_literal(text, word.clone())
                    .ok()
                    .as_deref()
                    == Some("-command")
            }) {
                rejects_script("lsort -command")
            } else {
                CommandEffect::OpaqueWorldHost
            }
        }
        "fcopy" | "regsub" => {
            if words.iter().skip(1).any(|word| {
                word_literal(text, word.clone())
                    .ok()
                    .as_deref()
                    == Some("-command")
            }) {
                rejects_script(&format!("{name} -command"))
            } else {
                CommandEffect::OpaqueWorldHost
            }
        }
        "chan" => match static_subcommand(text, words) {
            Some(subcommand) if subcommand == "event" => rejects_script("chan event"),
            Some(_) => CommandEffect::OpaqueWorldHost,
            None => rejects_script("chan with a dynamic subcommand"),
        },
        _ => CommandEffect::OpaqueWorldHost,
    }
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
        }
    }

    fn compile_unit(
        mut self,
        commands: &[&tcld_syntax::TclCommandSpan],
        unit_span: Range<usize>,
        args_spec: Option<&str>,
    ) -> Result<CompiledUnit, CompileDiagnostic> {
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
        let locals = args_spec
            .map(parse_simple_args)
            .transpose()
            .map_err(|reason| {
                diagnostic(self.path, self.source, unit_span.clone(), "proc", reason)
            })?
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .map(|(index, name)| CompiledLocal {
                name,
                default_value: None,
                frame_index: index as i32,
                flag_mask: 0,
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
                num_except_ranges: 0,
                num_aux_data_items: 0,
                num_cmd_loc_bytes: 0,
                max_except_depth: 0,
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
            exceptions: vec![],
            auxiliaries: vec![],
            proc_num_args: locals.len() as i32,
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
        let name = word_literal(text, words[0].clone())
            .map_err(|reason| diagnostic(self.path, self.source, span.clone(), text, reason))?;
        if name == "proc" {
            if !self.static_declarations {
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
            self.maps.push(VerifiedCommandMap {
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
        if self.mode == CompileMode::VmStrict && effect == CommandEffect::OpaqueWorldHost {
            return Err(diagnostic(
                self.path,
                self.source,
                span,
                text,
                format!("command {name:?} requires WorldHost and is not admitted by vm-strict"),
            ));
        }
        if let Some(signature) = self.proc_signature(&name) {
            let supplied = words.len() - 1;
            if supplied < signature.fixed || (!signature.variadic && supplied != signature.fixed) {
                return Err(diagnostic(
                    self.path,
                    self.source,
                    span,
                    text,
                    format!(
                        "wrong # args for {name}: expected {}{}, got {supplied}",
                        signature.fixed,
                        if signature.variadic { " or more" } else { "" }
                    ),
                ));
            }
        }
        if name == "expr" {
            self.compile_expr(text, &words, span.clone())?;
        } else if name == "return" {
            let (options, result_word) = match words.len() {
                1 => ("-code 0 -level 1".to_string(), None),
                2 => ("-code 0 -level 1".to_string(), Some(words[1].clone())),
                4 if word_literal(text, words[1].clone()).ok().as_deref() == Some("-code") => {
                    let code = word_literal(text, words[2].clone()).map_err(|reason| {
                        diagnostic(self.path, self.source, span.clone(), text, reason)
                    })?;
                    if !matches!(
                        code.as_str(),
                        "ok" | "error" | "return" | "break" | "continue"
                    ) && code.parse::<i32>().is_err()
                    {
                        return Err(diagnostic(
                            self.path,
                            self.source,
                            span,
                            text,
                            format!("unsupported return code {code:?}"),
                        ));
                    }
                    (format!("-code {code} -level 1"), Some(words[3].clone()))
                }
                _ => {
                    return Err(diagnostic(
                        self.path,
                        self.source,
                        span,
                        text,
                        "V1 return accepts `return ?-code code? ?result?`".into(),
                    ));
                }
            };
            self.push_literal(&options);
            if let Some(word) = result_word {
                self.compile_word(text, word, span.start, nesting_depth)?;
            } else {
                self.push_literal("");
            }
            self.emit(Opcode::ReturnStk);
        } else {
            for word in &words {
                self.compile_word(text, word.clone(), span.start, nesting_depth)?;
            }
            self.emit_arg(Opcode::InvokeStk1, words.len() as u32);
        }
        let (line, _) = line_column(self.source, span.start);
        let (end_line, _) = line_column(self.source, span.end.saturating_sub(1));
        self.maps.push(VerifiedCommandMap {
            source_span: span,
            start_line: line,
            end_line,
            anchor_pc: anchor,
            pc_range: anchor..self.code.len() as u32,
            nesting_depth,
        });
        Ok(())
    }

    fn proc_signature(&self, name: &str) -> Option<ProcSignature> {
        self.proc_signatures
            .get(name)
            .copied()
            .or_else(|| self.proc_signatures.get(&format!("::{name}")).copied())
    }

    fn compile_expr(
        &mut self,
        text: &str,
        words: &[Range<usize>],
        span: Range<usize>,
    ) -> Result<(), CompileDiagnostic> {
        if words.len() != 2 {
            return Err(diagnostic(
                self.path,
                self.source,
                span,
                text,
                "V1 expr requires one braced expression".into(),
            ));
        }
        let expr = braced_literal(text, words[1].clone())
            .map_err(|reason| diagnostic(self.path, self.source, span.clone(), text, reason))?;
        let parts = expr.split_whitespace().collect::<Vec<_>>();
        if parts.len() == 1 {
            self.compile_expr_atom(parts[0], span, text)?;
            return Ok(());
        }
        if parts.len() != 3 {
            return Err(diagnostic(
                self.path,
                self.source,
                span,
                text,
                "V1 expr supports an atom or binary expression with spaces".into(),
            ));
        }
        self.compile_expr_atom(parts[0], span.clone(), text)?;
        self.compile_expr_atom(parts[2], span.clone(), text)?;
        let opcode = match parts[1] {
            "+" => Opcode::Add,
            "-" => Opcode::Sub,
            "*" => Opcode::Mult,
            "/" => Opcode::Div,
            "==" => Opcode::Eq,
            "!=" => Opcode::Neq,
            "<" => Opcode::Lt,
            ">" => Opcode::Gt,
            "<=" => Opcode::Le,
            ">=" => Opcode::Ge,
            other => {
                return Err(diagnostic(
                    self.path,
                    self.source,
                    span,
                    text,
                    format!("unsupported V1 expr operator {other:?}"),
                ));
            }
        };
        self.emit(opcode);
        Ok(())
    }

    fn compile_expr_atom(
        &mut self,
        atom: &str,
        span: Range<usize>,
        command: &str,
    ) -> Result<(), CompileDiagnostic> {
        if let Some(name) = atom.strip_prefix('$') {
            if !valid_var_name(name) {
                return Err(diagnostic(
                    self.path,
                    self.source,
                    span,
                    command,
                    "unsupported expr variable".into(),
                ));
            }
            self.push_literal(name);
            self.emit(Opcode::LoadScalarStk);
        } else if atom.parse::<i64>().is_ok() {
            self.push_literal(atom);
        } else {
            return Err(diagnostic(
                self.path,
                self.source,
                span,
                command,
                format!("unsupported expr atom {atom:?}"),
            ));
        }
        Ok(())
    }

    fn compile_word(
        &mut self,
        command: &str,
        word: Range<usize>,
        command_start: usize,
        nesting_depth: u32,
    ) -> Result<(), CompileDiagnostic> {
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
                    return Err(diagnostic(
                        self.path,
                        self.source,
                        inner_base + cursor..inner_base + cursor + 1,
                        command,
                        "backslash substitution is not admitted by V1".into(),
                    ));
                }
                b'$' => {
                    if literal_start < cursor {
                        self.push_literal(&inner[literal_start..cursor]);
                        pieces += 1;
                    }
                    let end = variable_end(inner, cursor + 1);
                    let name = &inner[cursor + 1..end];
                    if !valid_var_name(name) {
                        return Err(diagnostic(
                            self.path,
                            self.source,
                            inner_base + cursor..inner_base + end,
                            command,
                            "unsupported scalar substitution".into(),
                        ));
                    }
                    self.push_literal(name);
                    self.emit(Opcode::LoadScalarStk);
                    pieces += 1;
                    cursor = end;
                    literal_start = cursor;
                }
                b'[' => {
                    if literal_start < cursor {
                        self.push_literal(&inner[literal_start..cursor]);
                        pieces += 1;
                    }
                    let close = matching_bracket(inner, cursor).ok_or_else(|| {
                        diagnostic(
                            self.path,
                            self.source,
                            inner_base + cursor..inner_base + inner.len(),
                            command,
                            "unterminated command substitution".into(),
                        )
                    })?;
                    let nested_span = inner_base + cursor + 1..inner_base + close;
                    if has_top_level_separator(&self.source[nested_span.clone()]) {
                        return Err(diagnostic(
                            self.path,
                            self.source,
                            nested_span,
                            command,
                            "V1 command substitution must contain exactly one command".into(),
                        ));
                    }
                    self.compile_command(nested_span, nesting_depth + 1)?;
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
        self.emit(opcode);
        let size = TCL_86_OPCODE_TABLE
            .iter()
            .position(|candidate| *candidate == opcode)
            .and_then(|index| tcld_core::inst_table::TCL_86_INSTRUCTION_TABLE.get(index))
            .and_then(|desc| desc.operands.first())
            .map(|op| op.mem_size())
            .unwrap_or(0);
        if size == 1 {
            self.code.push(arg as u8);
        } else {
            self.code.extend_from_slice(&arg.to_be_bytes());
        }
    }
}

fn opcode_byte(opcode: Opcode) -> u8 {
    TCL_86_OPCODE_TABLE
        .iter()
        .position(|candidate| *candidate == opcode)
        .expect("Tcl 8.6 opcode") as u8
}

fn parse_simple_args(spec: &str) -> Result<Vec<String>, String> {
    let words = scan_words(spec)?;
    let args = words
        .into_iter()
        .map(|span| {
            let name = word_literal(spec, span)?;
            if !valid_var_name(&name) {
                Err(format!("V1 proc argument {name:?} is not a simple scalar"))
            } else {
                Ok(name)
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut unique = std::collections::BTreeSet::new();
    for (index, name) in args.iter().enumerate() {
        if !unique.insert(name.as_str()) {
            return Err(format!("duplicate proc argument {name:?}"));
        }
        if name == "args" && index + 1 != args.len() {
            return Err("variadic `args` must be the final proc argument".into());
        }
    }
    Ok(args)
}

fn scan_words(text: &str) -> Result<Vec<Range<usize>>, String> {
    let bytes = text.as_bytes();
    let mut words = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i == bytes.len() {
            break;
        }
        let start = i;
        let mut brace = 0usize;
        let mut bracket = 0usize;
        let mut quote = false;
        while i < bytes.len() {
            match bytes[i] {
                b'\\' => i = (i + 2).min(bytes.len()),
                b'{' if !quote => {
                    brace += 1;
                    i += 1;
                }
                b'}' if !quote => {
                    if brace == 0 {
                        return Err("unmatched closing brace".into());
                    }
                    brace -= 1;
                    i += 1;
                }
                b'[' if brace == 0 => {
                    bracket += 1;
                    i += 1;
                }
                b']' if brace == 0 => {
                    if bracket == 0 {
                        return Err("unmatched closing bracket".into());
                    }
                    bracket -= 1;
                    i += 1;
                }
                b'"' if brace == 0 => {
                    quote = !quote;
                    i += 1;
                }
                byte if byte.is_ascii_whitespace() && brace == 0 && bracket == 0 && !quote => break,
                _ => i += 1,
            }
        }
        if brace != 0 || bracket != 0 || quote {
            return Err("unterminated word grouping".into());
        }
        words.push(start..i);
    }
    Ok(words)
}

fn word_literal(text: &str, span: Range<usize>) -> Result<String, String> {
    let raw = &text[span];
    if raw.starts_with('{') {
        braced_literal_raw(raw)
    } else if raw.starts_with('"') {
        if raw.len() < 2 || !raw.ends_with('"') {
            Err("unterminated quoted word".into())
        } else if raw[1..raw.len() - 1].contains(['$', '[', '\\']) {
            Err("word must be static".into())
        } else {
            Ok(raw[1..raw.len() - 1].to_string())
        }
    } else if raw.contains(['$', '[', '\\']) {
        Err("word must be static".into())
    } else {
        Ok(raw.to_string())
    }
}

fn braced_literal(text: &str, span: Range<usize>) -> Result<String, String> {
    braced_literal_raw(&text[span])
}
fn braced_literal_raw(raw: &str) -> Result<String, String> {
    if raw.len() < 2 || !raw.starts_with('{') || !raw.ends_with('}') {
        Err("expected braced literal".into())
    } else {
        Ok(raw[1..raw.len() - 1].to_string())
    }
}
fn valid_var_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b':'))
}
fn variable_end(text: &str, start: usize) -> usize {
    text.as_bytes()[start..]
        .iter()
        .position(|b| !(b.is_ascii_alphanumeric() || matches!(b, b'_' | b':')))
        .map_or(text.len(), |offset| start + offset)
}
fn matching_bracket(text: &str, open: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut brace = 0usize;
    for (i, byte) in bytes.iter().enumerate().skip(open) {
        match *byte {
            b'{' => brace += 1,
            b'}' => brace = brace.saturating_sub(1),
            b'[' if brace == 0 => depth += 1,
            b']' if brace == 0 => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}
fn has_top_level_separator(text: &str) -> bool {
    let mut brace = 0usize;
    let mut bracket = 0usize;
    let mut quote = false;
    for byte in text.bytes() {
        match byte {
            b'{' if !quote => brace += 1,
            b'}' if !quote => brace = brace.saturating_sub(1),
            b'[' if brace == 0 => bracket += 1,
            b']' if brace == 0 => bracket = bracket.saturating_sub(1),
            b'"' if brace == 0 => quote = !quote,
            b';' | b'\n' if brace == 0 && bracket == 0 && !quote => return true,
            _ => {}
        }
    }
    false
}
fn line_column(source: &str, offset: usize) -> (u32, u32) {
    let prefix = &source[..offset.min(source.len())];
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
    identity.update(source.as_bytes());
    format!("{:x}", identity.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let mut parsed = tcld_syntax::command_spans(source)
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
            "puts hello\n".into(),
            CompileMode::VmStrict,
        )
        .unwrap_err();
        let rendered = error.to_string();
        assert!(rendered.contains("/tmp/demo.tcl:1:1"));
        assert!(rendered.contains("WorldHost"));
        assert!(rendered.contains("--backend=host"));
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
            ("source", "source child.tcl\n"),
            ("eval", "eval {set hidden 1}\n"),
            ("if", "if {1} {set hidden 1}\n"),
            ("while", "while {0} {set hidden 1}\n"),
            ("namespace eval", "namespace eval demo {set hidden 1}\n"),
            ("apply", "apply {{} {set hidden 1}}\n"),
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
                "set x 1\n"
            )
        );
        assert_ne!(
            image.image_digest,
            image_identity_digest(
                DEBUG_IMAGE_SCHEMA,
                "different-policy",
                image.mode,
                "set x 1\n"
            )
        );
        assert_ne!(
            image.image_digest,
            image_identity_digest(
                DEBUG_IMAGE_SCHEMA,
                COMPILER_SEMANTICS,
                CompileMode::VmStrict,
                "set x 1\n"
            )
        );
        assert_ne!(
            image.image_digest,
            image_identity_digest(
                DEBUG_IMAGE_SCHEMA,
                COMPILER_SEMANTICS,
                image.mode,
                "set x 2\n"
            )
        );

        let mut corrupt_source_digest = image.clone();
        corrupt_source_digest.source.content_sha256 = "0".repeat(64);
        assert!(corrupt_source_digest.verify().is_err());

        let mut corrupt_image_digest = image.clone();
        corrupt_image_digest.image_digest = "0".repeat(64);
        assert!(corrupt_image_digest.verify().is_err());
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

        let default = compile_source(
            PathBuf::from("/tmp/default.tcl"),
            "proc greet {{name world}} { return $name }\n".into(),
            CompileMode::VmStrict,
        )
        .unwrap_err();
        assert!(default.reason.contains("simple scalar"), "{default}");

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
