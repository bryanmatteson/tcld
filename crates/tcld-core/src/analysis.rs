use crate::bytecode::{AuxData, AuxDataValue, Instruction};
use crate::inst_table::{FROM_OP1, Opcode};
use crate::util::truncate;
use std::borrow::Cow;
use std::collections::{BTreeSet, HashMap, VecDeque};

#[derive(Debug, Clone)]
pub struct BasicBlock {
    pub id: usize,
    pub start: usize,
    pub end: usize,
    pub edges: Vec<usize>,
}

/// Interned stack value — avoids allocating the same tag string thousands of
/// times during trace-frame interpretation. Static tags like `"<underflow>"`
/// and phi-merge markers use `Cow::Borrowed`, while instruction-derived values
/// are `Cow::Owned`.
type StackValue = Cow<'static, str>;

#[derive(Debug, Clone)]
pub struct TraceFrame {
    pub instruction_index: usize,
    pub pc: i32,
    pub opcode: String,
    pub argc: i32,
    pub stack_delta: i32,
    pub before_depth: i32,
    pub after_depth: i32,
    pub popped: Vec<String>,
    pub pushed: Vec<String>,
    pub next_locations: Vec<i32>,
}

pub fn build_basic_blocks(instructions: &[Instruction]) -> Vec<BasicBlock> {
    build_basic_blocks_with_auxiliaries(instructions, &[])
}

pub fn build_basic_blocks_with_auxiliaries(
    instructions: &[Instruction],
    auxiliaries: &[AuxData],
) -> Vec<BasicBlock> {
    if instructions.is_empty() {
        return Vec::new();
    }

    let mut loc_to_idx = HashMap::new();
    for (idx, inst) in instructions.iter().enumerate() {
        loc_to_idx.insert(inst.location, idx);
    }

    let mut leaders = BTreeSet::new();
    leaders.insert(0usize);
    for (idx, inst) in instructions.iter().enumerate() {
        let fallthrough = inst.location + inst.size();
        let next_locations = instruction_next_locations(inst, auxiliaries);
        for next in &next_locations {
            if *next == fallthrough {
                continue;
            }
            if let Some(target_idx) = loc_to_idx.get(next).copied() {
                leaders.insert(target_idx);
            }
        }
        if ends_basic_block(inst, &next_locations) && idx + 1 < instructions.len() {
            leaders.insert(idx + 1);
        }
    }

    let leader_indices = leaders.into_iter().collect::<Vec<_>>();
    let mut blocks = Vec::new();
    for (block_id, start_idx) in leader_indices.iter().copied().enumerate() {
        let end_idx = if block_id + 1 < leader_indices.len() {
            leader_indices[block_id + 1].saturating_sub(1)
        } else {
            instructions.len() - 1
        };
        blocks.push(BasicBlock {
            id: block_id,
            start: start_idx,
            end: end_idx,
            edges: Vec::new(),
        });
    }

    let mut idx_to_block = vec![0usize; instructions.len()];
    for block in &blocks {
        for block_idx in idx_to_block
            .iter_mut()
            .take(block.end + 1)
            .skip(block.start)
        {
            *block_idx = block.id;
        }
    }

    for block in &mut blocks {
        let mut edges = BTreeSet::new();
        let last_inst = &instructions[block.end];
        for next in instruction_next_locations(last_inst, auxiliaries) {
            if let Some(target_idx) = loc_to_idx.get(&next).copied() {
                edges.insert(idx_to_block[target_idx]);
            }
        }
        block.edges = edges.into_iter().collect();
    }

    blocks
}

pub fn interpret_trace_frames(instructions: &[Instruction]) -> Vec<TraceFrame> {
    interpret_trace_frames_with_auxiliaries(instructions, &[])
}

pub fn interpret_trace_frames_with_auxiliaries(
    instructions: &[Instruction],
    auxiliaries: &[AuxData],
) -> Vec<TraceFrame> {
    if instructions.is_empty() {
        return Vec::new();
    }

    let mut location_to_index = HashMap::new();
    for (idx, inst) in instructions.iter().enumerate() {
        location_to_index.insert(inst.location, idx);
    }

    let mut successors = vec![Vec::new(); instructions.len()];
    for (idx, inst) in instructions.iter().enumerate() {
        let mut next = Vec::new();
        for loc in instruction_next_locations(inst, auxiliaries) {
            if let Some(target_idx) = location_to_index.get(&loc).copied() {
                next.push(target_idx);
            }
        }
        successors[idx] = next;
    }

    let mut in_states: Vec<Option<Vec<StackValue>>> = vec![None; instructions.len()];
    in_states[0] = Some(Vec::new());
    let mut queue = VecDeque::new();
    queue.push_back(0usize);

    while let Some(idx) = queue.pop_front() {
        let Some(before_stack) = in_states[idx].clone() else {
            continue;
        };
        let step = simulate_trace_step(&instructions[idx], idx, &before_stack);
        for succ in &successors[idx] {
            if merge_stack_state(&mut in_states[*succ], &step.after_stack) {
                queue.push_back(*succ);
            }
        }
    }

    let mut frames = Vec::with_capacity(instructions.len());
    for (idx, inst) in instructions.iter().enumerate() {
        let before_stack = in_states[idx].clone().unwrap_or_default();
        let step = simulate_trace_step(inst, idx, &before_stack);
        frames.push(TraceFrame {
            instruction_index: idx,
            pc: inst.location,
            opcode: inst.name().to_string(),
            argc: inst.argc(),
            stack_delta: instruction_stack_delta(inst),
            before_depth: before_stack.len() as i32,
            after_depth: step.after_stack.len() as i32,
            popped: step.popped.into_iter().map(|v| v.into_owned()).collect(),
            pushed: step.pushed.into_iter().map(|v| v.into_owned()).collect(),
            next_locations: instruction_next_locations(inst, auxiliaries),
        });
    }

    frames
}

struct TraceStep {
    popped: Vec<StackValue>,
    pushed: Vec<StackValue>,
    after_stack: Vec<StackValue>,
}

fn simulate_trace_step(
    inst: &Instruction,
    instruction_index: usize,
    before_stack: &[StackValue],
) -> TraceStep {
    let mut working = before_stack.to_vec();
    let pop_count = inst.argc().max(0) as usize;
    let mut popped = Vec::with_capacity(pop_count);
    let available_pop = pop_count.min(working.len());
    for _ in 0..available_pop {
        if let Some(value) = working.pop() {
            popped.push(value);
        }
    }
    popped.reverse();
    if pop_count > available_pop {
        let mut with_underflow = Vec::with_capacity(pop_count);
        with_underflow.extend(std::iter::repeat_n(
            Cow::Borrowed("<underflow>"),
            pop_count - available_pop,
        ));
        with_underflow.extend(popped);
        popped = with_underflow;
    }

    let delta = instruction_stack_delta(inst);
    let push_count = (pop_count as i32 + delta).max(0) as usize;
    let mut pushed = Vec::new();
    for slot in 0..push_count {
        let value: StackValue = if let Some(v) = &inst.value {
            let mut repr = v.source_repr().replace('\n', "\\n");
            repr = truncate(&repr, 40, "...");
            Cow::Owned(repr)
        } else {
            Cow::Owned(format!(
                "{}#{}@{}",
                inst.name(),
                slot + 1,
                instruction_index
            ))
        };
        working.push(value.clone());
        pushed.push(value);
    }

    TraceStep {
        popped,
        pushed,
        after_stack: working,
    }
}

fn instruction_stack_delta(inst: &Instruction) -> i32 {
    if inst.opcode == Opcode::CallBuiltinFunc1 {
        1 - inst.argc().max(0)
    } else if inst.desc.stack_effect == FROM_OP1 {
        1 - inst.operands.first().copied().unwrap_or(0)
    } else {
        inst.desc.stack_effect
    }
}

pub fn instruction_next_locations(inst: &Instruction, auxiliaries: &[AuxData]) -> Vec<i32> {
    match inst.opcode {
        Opcode::Break | Opcode::Continue => inst
            .operands
            .first()
            .map(|offset| vec![inst.location + offset])
            .unwrap_or_default(),
        Opcode::JumpTable => {
            let mut targets = BTreeSet::new();
            targets.insert(inst.location + inst.size());
            if let Some(auxiliary) = inst
                .operands
                .first()
                .and_then(|index| usize::try_from(*index).ok())
                .and_then(|index| auxiliaries.get(index))
                && let AuxDataValue::JumpTable(table) = &auxiliary.value
            {
                targets.extend(table.dict.values().map(|offset| inst.location + offset));
            }
            targets.into_iter().collect()
        }
        Opcode::ReturnCodeBranch => [1, 3, 5, 7, 9]
            .into_iter()
            .map(|offset| inst.location + offset)
            .collect(),
        Opcode::Done
        | Opcode::ReturnImm
        | Opcode::ReturnStk
        | Opcode::Syntax
        | Opcode::Tailcall => Vec::new(),
        _ => inst.next_locations(),
    }
}

fn ends_basic_block(inst: &Instruction, next_locations: &[i32]) -> bool {
    matches!(inst.opcode, Opcode::JumpTable | Opcode::ReturnCodeBranch)
        || next_locations.len() != 1
        || next_locations[0] != inst.location + inst.size()
}

fn merge_stack_state(target: &mut Option<Vec<StackValue>>, incoming: &[StackValue]) -> bool {
    match target {
        None => {
            *target = Some(incoming.to_vec());
            true
        }
        Some(existing) => {
            let merged = merge_stack(existing, incoming);
            if *existing == merged {
                false
            } else {
                *existing = merged;
                true
            }
        }
    }
}

fn merge_stack(existing: &[StackValue], incoming: &[StackValue]) -> Vec<StackValue> {
    // A control-flow join with different stack depths is malformed bytecode or
    // an analysis-model mismatch. Retain only the values definitely present on
    // both paths. This is conservative and, critically, makes the abstract
    // state lattice finite even for an invalid positive-stack loop.
    let len = existing.len().min(incoming.len());
    let mut merged = Vec::with_capacity(len);
    for idx in 0..len {
        let left = existing.get(idx);
        let right = incoming.get(idx);
        if left == right {
            merged.push(
                left.cloned()
                    .unwrap_or_else(|| Cow::Owned(format!("<phi#{idx}>"))),
            );
        } else {
            merged.push(Cow::Owned(format!("<phi#{idx}>")));
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::{AuxDataType, JumptableInfo};
    use crate::inst_table::{BuiltInMathFunction, DEFAULT_INSTRUCTION_SET};
    use std::collections::BTreeMap;

    fn instruction(name: &str, location: i32, operands: Vec<i32>) -> Instruction {
        let (desc, opcode) = DEFAULT_INSTRUCTION_SET
            .instruction_table
            .iter()
            .copied()
            .zip(DEFAULT_INSTRUCTION_SET.opcode_table.iter().copied())
            .find(|(desc, _)| desc.name == name)
            .unwrap_or_else(|| panic!("missing instruction {name}"));
        Instruction::new(location, opcode, desc, operands, None)
    }

    #[test]
    fn basic_blocks_keep_straight_line_instructions_together() {
        let instructions = vec![
            instruction("push1", 0, vec![0]),
            instruction("push1", 2, vec![0]),
            instruction("jumpFalse1", 4, vec![6]),
            instruction("push1", 6, vec![0]),
            instruction("jump1", 8, vec![4]),
            instruction("push1", 10, vec![0]),
            instruction("done", 12, vec![]),
        ];

        let blocks = build_basic_blocks(&instructions);
        let ranges = blocks
            .iter()
            .map(|block| (block.start, block.end))
            .collect::<Vec<_>>();

        assert_eq!(ranges, vec![(0, 2), (3, 4), (5, 5), (6, 6)]);
        assert_eq!(blocks[0].edges, vec![1, 2]);
        assert_eq!(blocks[1].edges, vec![3]);
        assert_eq!(blocks[2].edges, vec![3]);
        assert!(blocks[3].edges.is_empty());
    }

    #[test]
    fn dynamic_stack_effect_replaces_arguments_with_one_result() {
        let invoke = instruction("invokeStk1", 0, vec![4]);
        assert_eq!(invoke.argc(), 4);
        assert_eq!(instruction_stack_delta(&invoke), -3);

        let before = (0..4)
            .map(|idx| Cow::Owned(format!("arg{idx}")))
            .collect::<Vec<_>>();
        let step = simulate_trace_step(&invoke, 0, &before);
        assert_eq!(step.popped.len(), 4);
        assert_eq!(step.pushed.len(), 1);
        assert_eq!(step.after_stack.len(), 1);
    }

    #[test]
    fn builtin_math_stack_effect_accounts_for_function_arity() {
        let pow = instruction("callBuiltinFunc1", 0, vec![BuiltInMathFunction::Pow as i32]);
        assert_eq!(pow.argc(), 2);
        assert_eq!(instruction_stack_delta(&pow), -1);
    }

    #[test]
    fn mismatched_join_depth_converges_to_definitely_present_values() {
        let mut target = Some(vec![Cow::Borrowed("a")]);
        let incoming = vec![Cow::Borrowed("a"), Cow::Borrowed("b")];

        assert!(!merge_stack_state(&mut target, &incoming));
        assert_eq!(target, Some(vec![Cow::Borrowed("a")]));
    }

    #[test]
    fn jump_table_uses_auxiliary_targets_and_fallthrough() {
        let jump_table = instruction("jumpTable", 10, vec![0]);
        let auxiliaries = vec![AuxData {
            kind: AuxDataType::JumpTable,
            value: AuxDataValue::JumpTable(JumptableInfo {
                dict: BTreeMap::from([("first".to_string(), 20), ("second".to_string(), 30)]),
            }),
        }];

        assert_eq!(
            instruction_next_locations(&jump_table, &auxiliaries),
            vec![15, 30, 40]
        );
        assert!(ends_basic_block(
            &jump_table,
            &instruction_next_locations(&jump_table, &[])
        ));
    }

    #[test]
    fn return_code_branch_exposes_all_runtime_successors() {
        let branch = instruction("returnCodeBranch", 100, vec![]);

        assert_eq!(
            instruction_next_locations(&branch, &[]),
            vec![101, 103, 105, 107, 109]
        );
    }
}
