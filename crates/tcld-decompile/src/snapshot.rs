use crate::analysis::{
    BasicBlock, TraceFrame, build_basic_blocks_with_auxiliaries, instruction_next_locations,
    interpret_trace_frames_with_auxiliaries,
};
use crate::bytecode::{ByteCode, ExceptionType, Instruction};
use crate::decompiler::source_map::SourceMap;
use crate::decompiler::{DecompiledCommandSpan, Decompiler};
use crate::diagnostics::DiagnosticsReport;
use crate::error::Result;
use crate::features::dump::{DumpOptions, dump_bytecode};
use crate::loader::{LoadInput, LoadProvenance, LoadedBytecode, load_bytecode};
use crate::source::SourceRepresentable;

#[derive(Debug, Clone)]
pub struct SnapshotMetadata {
    pub filename: String,
    pub path: String,
    pub tcl_version: String,
    pub instruction_count: usize,
    pub literal_count: usize,
    pub command_count: usize,
    pub exception_count: usize,
    pub aux_count: usize,
    pub code_bytes: i32,
    pub max_stack_depth: i32,
    pub format: Option<i32>,
    pub build: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct InstructionSnapshot {
    pub idx: usize,
    pub pc: i32,
    pub op: String,
    pub operands: Vec<i32>,
    pub argc: i32,
    pub group: Option<usize>,
    pub command_pc_range: Option<[i32; 2]>,
    pub group_start: bool,
    pub size: i32,
    pub is_jump: bool,
    pub is_jump_conditional: bool,
    pub is_jump_unconditional: bool,
    pub jump_target: Option<i32>,
    pub fallthrough: Option<i32>,
    pub primary_next_location: Option<i32>,
    pub secondary_next_location: Option<i32>,
    pub next_locations: Vec<i32>,
    pub comment: String,
}

#[derive(Debug, Clone)]
pub struct LiteralSnapshot {
    pub idx: usize,
    pub kind: String,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct ExceptionSnapshot {
    pub idx: usize,
    pub kind: String,
    pub nesting_level: i32,
    pub code_range: [i32; 2],
    pub break_offset: i32,
    pub continue_offset: i32,
    pub catch_offset: i32,
}

#[derive(Debug, Clone)]
pub struct DecompiledSnapshot {
    pub text: String,
    pub commands: Vec<DecompiledCommandSpan>,
}

#[derive(Clone)]
pub struct AnalysisSnapshot {
    pub metadata: SnapshotMetadata,
    pub instructions: Vec<InstructionSnapshot>,
    pub literals: Vec<LiteralSnapshot>,
    pub exceptions: Vec<ExceptionSnapshot>,
    pub basic_blocks: Vec<BasicBlock>,
    pub traces: Vec<TraceFrame>,
    pub decompile: DecompiledSnapshot,
    pub source_map: SourceMap,
    pub diagnostics: DiagnosticsReport,
    pub coverage: crate::coverage::CoverageMatrix,
    pub provenance: LoadProvenance,
    pub original_source: Option<String>,
    pub dump_text: String,
    pub summary_text: String,
    pub bytecode: ByteCode,
}

impl AnalysisSnapshot {
    pub fn from_input(input: LoadInput) -> Result<Self> {
        let loaded = load_bytecode(input)?;
        Self::from_loaded(loaded)
    }

    pub fn from_loaded(loaded: LoadedBytecode) -> Result<Self> {
        crate::init::ensure_initialized();
        let instructions = loaded.bytecode.get_instructions()?;
        let basic_blocks =
            build_basic_blocks_with_auxiliaries(&instructions, &loaded.bytecode.auxiliaries);
        let traces =
            interpret_trace_frames_with_auxiliaries(&instructions, &loaded.bytecode.auxiliaries);
        let groups = build_instruction_group_map(&instructions, &loaded.bytecode);
        let decompiled =
            match Decompiler::new(loaded.bytecode.clone())?.get_output_with_diagnostics() {
                Ok(report) => report,
                Err(_) => Decompiler::new(loaded.bytecode.clone())?
                    .get_output_approximate_with_diagnostics()?,
            };
        let source_map = SourceMap::from_decompiled_output(
            &loaded.bytecode,
            &decompiled.output,
            loaded.provenance.original_source_available,
        );
        let metadata = metadata_for(&loaded, &instructions);
        let dump_text = dump_bytecode(
            &loaded.bytecode,
            DumpOptions {
                show_all_literals: true,
                literal_filter: None,
            },
        )?;
        let summary_text = build_summary(&metadata);

        Ok(Self {
            metadata,
            instructions: instructions
                .iter()
                .enumerate()
                .map(|(idx, inst)| instruction_snapshot(idx, inst, &groups, &loaded.bytecode))
                .collect(),
            literals: loaded
                .bytecode
                .literals
                .iter()
                .enumerate()
                .map(|(idx, lit)| LiteralSnapshot {
                    idx,
                    kind: format!("{:?}", lit.kind),
                    value: lit.source_repr(),
                })
                .collect(),
            exceptions: loaded
                .bytecode
                .exceptions
                .iter()
                .enumerate()
                .map(|(idx, ex)| ExceptionSnapshot {
                    idx,
                    kind: match ex.kind {
                        ExceptionType::Catch => "Catch".to_string(),
                        ExceptionType::Loop => "Loop".to_string(),
                    },
                    nesting_level: ex.nesting_level,
                    code_range: [ex.code_offset, ex.code_offset + ex.num_code_bytes],
                    break_offset: ex.break_offset,
                    continue_offset: ex.continue_offset,
                    catch_offset: ex.catch_offset,
                })
                .collect(),
            basic_blocks,
            traces,
            decompile: DecompiledSnapshot {
                text: decompiled.output.text,
                commands: decompiled.output.commands,
            },
            source_map,
            diagnostics: decompiled.diagnostics,
            coverage: crate::coverage::generate_coverage_matrix(Default::default()),
            original_source: loaded.original_source.clone(),
            provenance: loaded.provenance,
            dump_text,
            summary_text,
            bytecode: loaded.bytecode,
        })
    }
}

fn metadata_for(loaded: &LoadedBytecode, instructions: &[Instruction]) -> SnapshotMetadata {
    let filename = loaded
        .provenance
        .normalized_path
        .as_ref()
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or(&loaded.provenance.label)
        .to_string();
    let tcl_version = loaded
        .bytecode
        .header
        .as_ref()
        .map(|h| format!("{}.{}", h.tcl_version.major, h.tcl_version.minor))
        .unwrap_or_else(|| "unknown".to_string());
    SnapshotMetadata {
        filename,
        path: loaded.provenance.label.clone(),
        tcl_version,
        instruction_count: instructions.len(),
        literal_count: loaded.bytecode.literals.len(),
        command_count: loaded.bytecode.cmd_code_ranges.len(),
        exception_count: loaded.bytecode.exceptions.len(),
        aux_count: loaded.bytecode.auxiliaries.len(),
        code_bytes: loaded.bytecode.info.num_code_bytes,
        max_stack_depth: loaded.bytecode.info.max_stack_depth,
        format: loaded.bytecode.header.as_ref().map(|h| h.format),
        build: loaded.bytecode.header.as_ref().map(|h| h.build),
    }
}

fn instruction_snapshot(
    idx: usize,
    inst: &Instruction,
    groups: &[Option<usize>],
    bytecode: &ByteCode,
) -> InstructionSnapshot {
    let group = groups[idx];
    let group_start = idx == 0 || groups[idx] != groups[idx - 1];
    let command_pc_range = group
        .and_then(|group| bytecode.cmd_code_ranges.get(group))
        .map(|range| [range.start, range.end]);
    let comment = if inst.is_jump() {
        inst.jump_target()
            .map(|target| format!("pc {target}"))
            .unwrap_or_default()
    } else {
        inst.value
            .as_ref()
            .map(|value| value.source_repr().replace('\n', "\\n"))
            .unwrap_or_default()
    };
    InstructionSnapshot {
        idx,
        pc: inst.location,
        op: inst.name().to_string(),
        operands: inst.operands.clone(),
        argc: inst.argc(),
        group,
        command_pc_range,
        group_start,
        size: inst.size(),
        is_jump: inst.is_jump(),
        is_jump_conditional: inst.is_jump_conditional(),
        is_jump_unconditional: inst.is_jump_unconditional(),
        jump_target: inst.jump_target(),
        fallthrough: inst.fallthrough_location(),
        primary_next_location: inst.primary_next_location(),
        secondary_next_location: inst.secondary_next_location(),
        next_locations: instruction_next_locations(inst, &bytecode.auxiliaries),
        comment,
    }
}

pub fn build_instruction_group_map(
    instructions: &[Instruction],
    bytecode: &ByteCode,
) -> Vec<Option<usize>> {
    let mut groups = vec![None; instructions.len()];
    let mut cursor = 0usize;
    for (idx, inst) in instructions.iter().enumerate() {
        while cursor < bytecode.cmd_code_ranges.len()
            && inst.location >= bytecode.cmd_code_ranges[cursor].end
        {
            cursor += 1;
        }
        groups[idx] = if cursor < bytecode.cmd_code_ranges.len()
            && bytecode.cmd_code_ranges[cursor].contains(&inst.location)
        {
            Some(cursor)
        } else {
            None
        };
    }
    groups
}

fn build_summary(metadata: &SnapshotMetadata) -> String {
    format!(
        "snapshot: canonical-domain\npath: {}\nheader: tcl={}\nmode: bytecode commands={} code_bytes={} instructions={} literals={} exceptions={} aux={}\n",
        metadata.path,
        metadata.tcl_version,
        metadata.command_count,
        metadata.code_bytes,
        metadata.instruction_count,
        metadata.literal_count,
        metadata.exception_count,
        metadata.aux_count
    )
}
