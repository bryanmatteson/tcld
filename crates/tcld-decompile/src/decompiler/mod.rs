use crate::bytecode::{AuxDataValue, ByteCode, ExceptionType, Instruction};
use crate::code::{
    ArrayElem, CmdBody, Code, CodeRef, CommandKind, cmd_array_ref, cmd_catch, cmd_dict_update,
    cmd_expanded_arg, cmd_expr, cmd_expr_func, cmd_for_loop, cmd_foreach, cmd_if_else, cmd_literal,
    cmd_op, cmd_proc_call, cmd_set, cmd_strcat, cmd_switch, cmd_variable, cmd_while_loop,
};
use crate::code_graph::CodeGraph;
use crate::error::{Result, TcldError};
use crate::inst_table::{BuiltInMathFunction, Opcode};
use crate::source::{SourceRef, sr, sr_dyn};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::Arc;

mod opcode_handlers;
mod passes;
mod reduce;

pub mod builtin_passes;
pub mod pass_registry;
pub mod pass_trait;
pub mod source_map;

/// A command the tcld compiler traced inline (`VerifiedCommandMap` with
/// `trace_inline` in tcld-compile). Its words are evaluated and packed by
/// `list argv_count` ahead of the `Nop` at `trace_pc`, where the VM pops them
/// as the command's argv; the implementation then runs up to the `Nop` at
/// `trace_end_pc`. The source command is exactly that argv, so the decompiler
/// renders the region from its words and treats the rest as instrumentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InlineTraceCapture {
    pub anchor_pc: i32,
    pub trace_pc: i32,
    pub trace_end_pc: i32,
    pub argv_count: usize,
}

pub struct Decompiler {
    bytecode: ByteCode,
    graph: CodeGraph,
    fastpath_suppress: Vec<Range<i32>>,
    trace_captures: Vec<InlineTraceCapture>,
    is_proc_body: bool,
    pipeline_config: DecompilePipelineConfig,
    pass_trace: Vec<crate::diagnostics::PassDecision>,
    pipeline_error: Option<String>,
    nested_render_issues: Vec<String>,
}

/// Identity of a decompiled command span.
///
/// The decompiler emits two flavors of spans:
///
/// * `Bytecode` spans correspond to a real entry in `ByteCode::cmd_code_ranges`
///   and are addressable by program counter — clicking an instruction can map
///   back to the same command.
/// * `Synthetic` spans cover commands that exist only in the rendered source
///   text (most commonly the bodies of `tbcload::bcproc` / `proc` literals,
///   whose bytecode does not carry per-command ranges of its own). Synthetic
///   identity is keyed by the enclosing proc's qualified name plus the byte
///   offset of the command within that proc body source — the same key the
///   runtime can reproduce from a `[info frame]` lookup.
/// * `Unknown` is the fallback when neither could be determined (rare; the
///   line-backed backfill uses it when no enclosing span exists at all).
#[derive(Debug, Clone)]
pub enum CommandIdentity {
    Bytecode {
        command_group: usize,
    },
    Synthetic {
        /// Qualified name of the enclosing proc (e.g. `"fixture::startlog"`), or
        /// `None` when the lift hasn't bound it yet (inner decompiles emit
        /// `None`; the outer-frame lift fills it in via `take_inner_commands`).
        proc_qname: Option<Arc<str>>,
        /// Byte offset of the command within the proc body source string.
        body_byte_offset: usize,
        /// Byte length of the command in the proc body source string.
        body_byte_len: usize,
        /// Command-shaped node kind (see `tcld_syntax::TclCommandKind`).
        kind: tcld_syntax::TclCommandKind,
        /// First word of the command, or canonical keyword for special forms.
        name: Option<Arc<str>>,
    },
    Unknown,
}

impl CommandIdentity {
    /// Bytecode-backed group index, or `None` for synthetic/unknown spans.
    pub fn command_group(&self) -> Option<usize> {
        match self {
            CommandIdentity::Bytecode { command_group } => Some(*command_group),
            _ => None,
        }
    }

    pub fn is_bytecode(&self) -> bool {
        matches!(self, CommandIdentity::Bytecode { .. })
    }

    pub fn is_synthetic(&self) -> bool {
        matches!(self, CommandIdentity::Synthetic { .. })
    }
}

#[derive(Debug, Clone)]
pub struct DecompiledCommandSpan {
    pub code_range: Range<i32>,
    pub text: String,
    pub start_line: usize,
    pub end_line: usize,
    pub identity: CommandIdentity,
    /// Root bytecode command group that owns this span when it came from a
    /// nested source body. Set by the outer-frame lift in
    /// `take_inner_commands`; `None` for top-level spans.
    pub parent_command_group: Option<usize>,
    /// Stable zero-based index of this span within its parent source body.
    pub nested_command_index: Option<usize>,
}

impl DecompiledCommandSpan {
    /// Convenience accessor for callers that only care about the bytecode
    /// group (instruction → source mapping). Equivalent to
    /// `self.identity.command_group()`.
    pub fn command_group(&self) -> Option<usize> {
        self.identity.command_group()
    }
}

#[derive(Debug, Clone, Default)]
pub struct DecompiledOutput {
    pub text: String,
    pub commands: Vec<DecompiledCommandSpan>,
}

pub fn align_command_ranges_to_spans(
    command_ranges: &[Range<i32>],
    spans: &[DecompiledCommandSpan],
) -> Vec<Option<usize>> {
    let n = command_ranges.len();
    let m = spans.len();
    if n == 0 {
        return Vec::new();
    }
    if m == 0 {
        return vec![None; n];
    }

    let skip_group_penalty: i64 = -25;
    let skip_span_penalty: i64 = -5;
    let neg_inf: i64 = i64::MIN / 4;

    let mut dp = vec![vec![neg_inf; m + 1]; n + 1];
    let mut choice = vec![vec![' '; m + 1]; n + 1];
    dp[0][0] = 0;
    choice[0][0] = 'S';

    for i in 1..=n {
        dp[i][0] = dp[i - 1][0] + skip_group_penalty;
        choice[i][0] = 'G';
    }
    for j in 1..=m {
        dp[0][j] = dp[0][j - 1] + skip_span_penalty;
        choice[0][j] = 'P';
    }

    for i in 1..=n {
        for j in 1..=m {
            let mut best = dp[i - 1][j] + skip_group_penalty;
            let mut ch = 'G';

            let skip_span = dp[i][j - 1] + skip_span_penalty;
            if skip_span > best {
                best = skip_span;
                ch = 'P';
            }

            let match_score =
                dp[i - 1][j - 1] + span_match_score(&command_ranges[i - 1], &spans[j - 1]);
            if match_score > best {
                best = match_score;
                ch = 'M';
            }

            dp[i][j] = best;
            choice[i][j] = ch;
        }
    }

    let mut mapping = vec![None; n];
    let mut i = n;
    let mut j = m;
    while i > 0 || j > 0 {
        match choice[i][j] {
            'M' => {
                mapping[i - 1] = Some(j - 1);
                i -= 1;
                j -= 1;
            }
            'G' => {
                mapping[i - 1] = None;
                i -= 1;
            }
            'P' => {
                j = j.saturating_sub(1);
            }
            _ => {
                if i > 0 {
                    i -= 1;
                } else {
                    j = j.saturating_sub(1);
                }
            }
        }
    }

    mapping
}

fn span_match_score(command_range: &Range<i32>, span: &DecompiledCommandSpan) -> i64 {
    let overlap = overlap_len(
        command_range.start,
        command_range.end,
        span.code_range.start,
        span.code_range.end,
    );
    let start_distance = (command_range.start - span.code_range.start).abs() as i64;
    let end_distance = (command_range.end - span.code_range.end).abs() as i64;
    if overlap > 0 {
        60 + (overlap * 4) - (start_distance / 2) - (end_distance / 2)
    } else {
        -((start_distance + end_distance) / 2)
    }
}

fn overlap_len(a_start: i32, a_end: i32, b_start: i32, b_end: i32) -> i64 {
    let start = a_start.max(b_start);
    let end = a_end.min(b_end);
    (end - start).max(0) as i64
}

#[derive(Clone)]
struct CodeSource(CodeRef);
#[derive(Clone)]
struct ExprCodeSource(CodeRef);

impl crate::source::SourceRepresentable for CodeSource {
    fn source_repr(&self) -> String {
        let mut out = String::new();
        self.write_source_repr(&mut out);
        out
    }

    fn write_source_repr(&self, out: &mut String) {
        self.0.borrow().write_source_repr(out);
    }
}

impl crate::source::SourceRepresentable for ExprCodeSource {
    fn source_repr(&self) -> String {
        let mut out = String::new();
        self.write_source_repr(&mut out);
        out
    }

    fn write_source_repr(&self, out: &mut String) {
        self.0.borrow().write_expr_repr(out);
    }
}

fn code_as_source(code: &CodeRef) -> SourceRef {
    sr_dyn(CodeSource(code.clone()))
}

fn code_as_expr(code: &CodeRef) -> SourceRef {
    sr_dyn(ExprCodeSource(code.clone()))
}

#[derive(Clone, Copy, Debug)]
enum PipelinePass {
    Instructions,
    DictUpdate,
    SwitchTables,
    Catches,
    ChainedLogicals,
    Foreach,
    IfElseFixpoint,
    LoopsFixpoint,
}

#[derive(Clone, Default)]
pub struct DecompilePipelineConfig {
    disabled_passes: BTreeSet<String>,
    custom_passes: Vec<Arc<dyn pass_trait::DecompilerPass>>,
    include_global_registry: bool,
}

impl DecompilePipelineConfig {
    pub fn disable_pass(mut self, name: impl Into<String>) -> Self {
        self.disabled_passes.insert(name.into());
        self
    }

    pub fn with_custom_registry(mut self, registry: pass_registry::PassRegistry) -> Self {
        self.custom_passes = registry.passes().to_vec();
        self.include_global_registry = false;
        self
    }

    /// Opt into process-global custom passes.
    ///
    /// The default pipeline is isolated and deterministic. Applications that
    /// intentionally use `register_pass` must enable this explicitly.
    pub fn with_global_registry(mut self) -> Self {
        self.include_global_registry = true;
        self
    }

    fn is_enabled(&self, name: &str) -> bool {
        !self.disabled_passes.contains(name)
    }
}

#[derive(Clone)]
enum PipelineEntry {
    BuiltIn {
        pass: PipelinePass,
        name: &'static str,
        priority: u32,
        fixpoint: bool,
    },
    Custom(Arc<dyn pass_trait::DecompilerPass>),
}

impl PipelineEntry {
    fn name(&self) -> &str {
        match self {
            Self::BuiltIn { name, .. } => name,
            Self::Custom(pass) => pass.name(),
        }
    }

    fn priority(&self) -> u32 {
        match self {
            Self::BuiltIn { priority, .. } => *priority,
            Self::Custom(pass) => pass.priority(),
        }
    }
}

mod core;
fn is_switch_jump_opcode(opcode: Opcode) -> bool {
    opcode == Opcode::JumpTable
}

fn is_ir_ignorable_inst(opcode: Opcode) -> bool {
    matches!(opcode, Opcode::StartCmd | Opcode::Nop)
}

fn is_reduction_separator_inst(opcode: Opcode) -> bool {
    is_ir_ignorable_inst(opcode)
        || matches!(
            opcode,
            Opcode::BeginCatch4
                | Opcode::EndCatch
                | Opcode::Jump1
                | Opcode::Jump4
                | Opcode::JumpTrue1
                | Opcode::JumpTrue4
                | Opcode::JumpFalse1
                | Opcode::JumpFalse4
                | Opcode::TryCvtToNumeric
                | Opcode::ExpandStart
                | Opcode::ExpandDrop
                | Opcode::PushReturnCode
                | Opcode::PushReturnOptions
                | Opcode::PushResult
                | Opcode::ReturnCodeBranch
                | Opcode::Reverse
                | Opcode::DictFirst
                | Opcode::DictNext
                | Opcode::DictDone
                | Opcode::DictExpand
                | Opcode::DictRecombineStk
                | Opcode::DictRecombineImm
                | Opcode::DictVerify
                | Opcode::LmapCollect
        )
}

fn expression_map(op: Opcode) -> Option<&'static str> {
    match op {
        Opcode::Gt => Some(">"),
        Opcode::Lt => Some("<"),
        Opcode::Ge => Some(">="),
        Opcode::Le => Some("<="),
        Opcode::Eq => Some("=="),
        Opcode::StrEq => Some("eq"),
        Opcode::StrLt => Some("lt"),
        Opcode::StrGt => Some("gt"),
        Opcode::StrLe => Some("le"),
        Opcode::StrGe => Some("ge"),
        Opcode::Neq => Some("!="),
        Opcode::Add => Some("+"),
        Opcode::Not => Some("!"),
        Opcode::Land => Some("&&"),
        Opcode::Lor => Some("||"),
        Opcode::Bitxor => Some("^"),
        Opcode::Bitor => Some("|"),
        Opcode::Bitnot => Some("~"),
        Opcode::Bitand => Some("&"),
        Opcode::Lshift => Some("<<"),
        Opcode::Rshift => Some(">>"),
        Opcode::Uminus => Some("-"),
        Opcode::Uplus => Some("+"),
        Opcode::Sub => Some("-"),
        Opcode::Mult => Some("*"),
        Opcode::Div => Some("/"),
        Opcode::Mod => Some("%"),
        Opcode::Expon => Some("**"),
        _ => None,
    }
}

fn expanded_arg(value: SourceRef) -> SourceRef {
    let mut out = String::new();
    out.push_str("{*}");
    value.write_repr(&mut out);
    sr(out)
}

fn idx_to_source(idx: i32) -> SourceRef {
    const TCL_INDEX_END: i32 = -2;
    if idx <= TCL_INDEX_END {
        let offset = TCL_INDEX_END - idx;
        if offset == 0 {
            return sr("end");
        }
        return sr(format!("end-{}", offset));
    }
    sr(idx)
}

fn clock_read_name(code: i32) -> Option<&'static str> {
    match code {
        0 => Some("clicks"),
        1 => Some("microseconds"),
        2 => Some("milliseconds"),
        3 => Some("seconds"),
        4 => Some("monotonic"),
        _ => None,
    }
}

fn return_code_name(code: i32) -> SourceRef {
    match code {
        0 => sr("ok"),
        1 => sr("error"),
        2 => sr("return"),
        3 => sr("break"),
        4 => sr("continue"),
        _ => sr(code),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::ByteCodeInfo;
    use crate::source::SourceRepresentable;

    fn empty_bytecode() -> ByteCode {
        ByteCode {
            header: None,
            info: ByteCodeInfo {
                num_commands: 0,
                num_src_bytes: 0,
                num_code_bytes: 0,
                num_lit_objects: 0,
                num_except_ranges: 0,
                num_aux_data_items: 0,
                num_cmd_loc_bytes: 0,
                max_except_depth: 0,
                max_stack_depth: 0,
                code_delta_size: 0,
                code_length_size: 0,
                src_delta_size: 0,
                src_length_size: 0,
            },
            code_bytes: Vec::new(),
            cmd_code_ranges: Vec::new(),
            cmd_source_ranges: None,
            literals: Vec::new(),
            exceptions: Vec::new(),
            auxiliaries: Vec::new(),
            proc_num_args: 0,
            proc_num_locals: 0,
            locals: Vec::new(),
        }
    }

    #[test]
    fn switch_opcode_helper_accepts_the_versioned_switch_opcode() {
        assert!(is_switch_jump_opcode(Opcode::JumpTable));
    }

    #[test]
    fn empty_bytecode_decompiles_to_empty_source() {
        let decompiler = Decompiler::new(empty_bytecode()).expect("decompiler created");
        let output = decompiler.get_source().expect("decompile succeeds");
        assert_eq!(output, "");
    }

    #[test]
    fn return_code_name_maps_code_2_to_return_keyword() {
        assert!(return_code_name(2).repr_eq_str("return"));
    }

    fn span_at(
        group: usize,
        code_range: Range<i32>,
        text: &str,
        line: usize,
    ) -> DecompiledCommandSpan {
        DecompiledCommandSpan {
            code_range,
            text: text.to_string(),
            start_line: line,
            end_line: line,
            identity: CommandIdentity::Bytecode {
                command_group: group,
            },
            parent_command_group: None,
            nested_command_index: None,
        }
    }

    #[test]
    fn align_command_ranges_preserves_order() {
        let ranges = vec![0..10, 10..20, 20..30];
        let spans = vec![
            span_at(0, 0..9, "a", 0),
            span_at(1, 10..19, "b", 1),
            span_at(2, 20..29, "c", 2),
        ];
        assert_eq!(
            align_command_ranges_to_spans(&ranges, &spans),
            vec![Some(0), Some(1), Some(2)]
        );
    }

    #[test]
    fn align_command_ranges_can_leave_group_unmapped() {
        let ranges = vec![0..10, 10..20];
        let spans = vec![span_at(0, 0..9, "a", 0)];
        let mapped = align_command_ranges_to_spans(&ranges, &spans);
        assert_eq!(mapped.len(), 2);
        assert_eq!(mapped[0], Some(0));
        assert_eq!(mapped[1], None);
    }

    #[test]
    fn public_identity_alignment_and_helper_boundaries_are_complete() {
        let synthetic = CommandIdentity::Synthetic {
            proc_qname: None,
            body_byte_offset: 0,
            body_byte_len: 1,
            kind: tcld_syntax::TclCommandKind::Command,
            name: None,
        };
        assert!(synthetic.is_synthetic());
        assert!(!synthetic.is_bytecode());
        assert_eq!(synthetic.command_group(), None);

        assert!(align_command_ranges_to_spans(&[], &[]).is_empty());
        let one_range = std::iter::once(0..1).collect::<Vec<_>>();
        assert_eq!(align_command_ranges_to_spans(&one_range, &[]), vec![None]);

        let config = DecompilePipelineConfig::default().with_global_registry();
        assert!(config.include_global_registry);

        let expression_cases = [
            (Opcode::StrEq, Some("eq")),
            (Opcode::StrLt, Some("lt")),
            (Opcode::StrGt, Some("gt")),
            (Opcode::StrLe, Some("le")),
            (Opcode::StrGe, Some("ge")),
            (Opcode::Bitxor, Some("^")),
            (Opcode::Bitor, Some("|")),
            (Opcode::Bitnot, Some("~")),
            (Opcode::Bitand, Some("&")),
            (Opcode::Lshift, Some("<<")),
            (Opcode::Rshift, Some(">>")),
            (Opcode::Uplus, Some("+")),
            (Opcode::Sub, Some("-")),
            (Opcode::Div, Some("/")),
            (Opcode::Expon, Some("**")),
            (Opcode::Done, None),
        ];
        for (opcode, expected) in expression_cases {
            assert_eq!(expression_map(opcode), expected);
        }

        for (code, expected) in [
            (0, Some("clicks")),
            (1, Some("microseconds")),
            (2, Some("milliseconds")),
            (3, Some("seconds")),
            (4, Some("monotonic")),
            (5, None),
        ] {
            assert_eq!(clock_read_name(code), expected);
        }
        for (code, expected) in [
            (0, "ok"),
            (1, "error"),
            (2, "return"),
            (3, "break"),
            (4, "continue"),
            (9, "9"),
        ] {
            assert!(return_code_name(code).repr_eq_str(expected));
        }

        let desc = *crate::inst_table::DEFAULT_INSTRUCTION_SET
            .instruction_table
            .iter()
            .find(|desc| {
                crate::inst_table::DEFAULT_INSTRUCTION_SET.opcode_from_i32(desc.opcode)
                    == Some(Opcode::Done)
            })
            .expect("done descriptor");
        let child = crate::code::code_ref(Code::Instruction(crate::bytecode::Instruction::new(
            0,
            Opcode::Done,
            desc,
            Vec::new(),
            None,
        )));
        let literal = cmd_literal(vec![child], sr("value"));
        assert_eq!(CodeSource(literal.clone()).source_repr(), "value");
        assert_eq!(ExprCodeSource(literal).source_repr(), "value");
    }
}
