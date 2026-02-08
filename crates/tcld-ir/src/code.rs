use crate::bytecode::Instruction;
use crate::source::{SourceRef, SourceRepresentable};
use crate::util::{right_justified, truncate};
use std::borrow::Cow;
use std::cell::{Ref, RefCell, RefMut};
use std::collections::BTreeMap;
use std::fmt;
use std::ops::Range;
use std::rc::{Rc, Weak};
use std::thread_local;

#[derive(Clone)]
pub struct CodeRef(Rc<RefCell<Code>>);

thread_local! {
    static CODE_NODES: RefCell<Vec<Weak<RefCell<Code>>>> = const { RefCell::new(Vec::new()) };
}

/// Drop stale node-accounting entries.
///
/// IR nodes are reference counted and are reclaimed automatically when the
/// owning graph and all derived `CodeRef` values are dropped. This function is
/// retained as a compatibility hook for long-running callers that want the
/// diagnostic registry compacted between sessions.
pub fn reset_code_arena() {
    CODE_NODES.with(|nodes| {
        nodes.borrow_mut().retain(|node| node.strong_count() > 0);
    });
}

/// Returns the number of currently live IR nodes on this thread.
pub fn code_arena_len() -> usize {
    CODE_NODES.with(|nodes| {
        nodes
            .borrow()
            .iter()
            .filter(|node| node.strong_count() > 0)
            .count()
    })
}

impl CodeRef {
    pub fn borrow(&self) -> Ref<'_, Code> {
        self.0.borrow()
    }

    pub fn borrow_mut(&self) -> RefMut<'_, Code> {
        self.0.borrow_mut()
    }

    pub fn ptr_eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl PartialEq for CodeRef {
    fn eq(&self, other: &Self) -> bool {
        self.ptr_eq(other)
    }
}

impl Eq for CodeRef {}

impl fmt::Debug for CodeRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("CodeRef")
            .field(&(Rc::as_ptr(&self.0) as usize))
            .finish()
    }
}

#[derive(Clone)]
pub struct ArrayElem {
    pub args: Vec<SourceRef>,
}

impl ArrayElem {
    pub fn new(args: Vec<SourceRef>) -> Self {
        ArrayElem { args }
    }
}

impl SourceRepresentable for ArrayElem {
    fn source_repr(&self) -> String {
        let a = self.args[0].repr_cow();
        let b = self.args[1].repr_cow();
        let mut out = String::with_capacity(a.len() + b.len() + 2);
        out.push_str(&a);
        out.push('(');
        out.push_str(&b);
        out.push(')');
        out
    }

    fn write_source_repr(&self, out: &mut String) {
        self.args[0].write_repr(out);
        out.push('(');
        self.args[1].write_repr(out);
        out.push(')');
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmdJumpType {
    Jump,
    Break,
    Continue,
}

#[derive(Clone)]
pub struct CmdJump {
    pub jump_type: CmdJumpType,
    pub target: i32,
    pub conditional: bool,
}

#[derive(Clone)]
pub struct BlockBase {
    pub location: i32,
    pub children: BTreeMap<i32, CodeRef>,
}

impl BlockBase {
    pub fn from_children(children: Vec<CodeRef>) -> Self {
        let mut map = BTreeMap::new();
        for child in children {
            let loc = child.borrow().location();
            map.insert(loc, child);
        }
        let location = *map.keys().next().expect("Block requires children");
        BlockBase {
            location,
            children: map,
        }
    }

    pub fn size(&self) -> i32 {
        self.children.values().map(|c| c.borrow().size()).sum()
    }

    pub fn for_each_next_location<F>(&self, f: &mut F)
    where
        F: FnMut(i32),
    {
        if let Some((_, last)) = self.children.iter().next_back() {
            last.borrow().for_each_next_location(f);
        }
    }

    pub fn next_locations(&self) -> Vec<i32> {
        if let Some((_, last)) = self.children.iter().next_back() {
            return last.borrow().next_locations();
        }
        vec![]
    }
}

#[derive(Clone)]
pub struct CommandBase {
    pub block: BlockBase,
    pub stackn: i32,
}

impl CommandBase {
    pub fn from_children(children: Vec<CodeRef>) -> Self {
        CommandBase {
            block: BlockBase::from_children(children),
            stackn: 1,
        }
    }

    pub fn merge(&mut self, codes: Vec<CodeRef>) {
        self.merge_slice(&codes);
    }

    pub fn merge_slice(&mut self, codes: &[CodeRef]) {
        for code in codes.iter().cloned() {
            let loc = code.borrow().location();
            self.block.children.insert(loc, code);
        }
    }

    pub fn merge_one(&mut self, code: CodeRef) {
        let loc = code.borrow().location();
        self.block.children.insert(loc, code);
    }

    pub fn destack(&mut self) {
        self.stackn -= 1;
    }
}

#[derive(Clone)]
pub struct CmdBody {
    pub block: BlockBase,
}

impl CmdBody {
    pub fn new(children: Vec<CodeRef>) -> Self {
        if children.is_empty() {
            CmdBody {
                block: BlockBase {
                    location: 0,
                    children: BTreeMap::new(),
                },
            }
        } else {
            CmdBody {
                block: BlockBase::from_children(children),
            }
        }
    }

    pub fn source_repr(&self) -> String {
        let mut out = String::new();
        self.write_source_repr(&mut out);
        out
    }

    pub fn write_source_repr(&self, out: &mut String) {
        if self.block.children.is_empty() {
            return;
        }
        if self.block.children.len() == 1 {
            let (_, code) = self.block.children.iter().next().unwrap();
            let rep = code.borrow().source_repr();
            if let Code::Command(cmd) = &*code.borrow()
                && matches!(&cmd.kind, CommandKind::Literal { .. })
                && rep.starts_with('{')
                && rep.ends_with('}')
                && rep.contains('\n')
            {
                out.push_str(&rep[1..rep.len() - 1]);
                return;
            }
            out.push_str(&rep);
            return;
        }
        let mut first_cmd = None;
        let mut cmd_count = 0usize;
        for code in self.block.children.values() {
            if code.borrow().is_command() {
                cmd_count += 1;
                if cmd_count == 1 {
                    first_cmd = Some(code.clone());
                } else {
                    break;
                }
            }
        }
        if cmd_count == 0 {
            return;
        }
        if cmd_count == 1 {
            let cmd_ref = first_cmd.expect("first command exists");
            let rep = cmd_ref.borrow().source_repr();
            if let Code::Command(cmd) = &*cmd_ref.borrow()
                && matches!(&cmd.kind, CommandKind::Literal { .. })
                && rep.starts_with('{')
                && rep.ends_with('}')
                && rep.contains('\n')
            {
                out.push_str(&rep[1..rep.len() - 1]);
                return;
            }
            out.push_str(&rep);
            return;
        }
        let mut first = true;
        let mut prev_break_after = false;
        for code in self.block.children.values() {
            if !code.borrow().is_command() {
                continue;
            }
            let mut rendered = String::new();
            code.borrow().write_source_repr(&mut rendered);
            if rendered.is_empty() {
                continue;
            }
            if !first {
                out.push('\n');
                if prev_break_after {
                    out.push('\n');
                }
            }
            first = false;
            out.push_str(&rendered);
            prev_break_after = command_needs_following_blank_line(code.clone());
        }
    }
}

fn command_needs_following_blank_line(code: CodeRef) -> bool {
    let borrowed = code.borrow();
    let Code::Command(cmd) = &*borrowed else {
        return false;
    };
    match &cmd.kind {
        CommandKind::ProcCall { args } => args
            .first()
            .is_some_and(|head| head.repr_eq_str("proc") || head.repr_eq_str("tbcload::bcproc")),
        CommandKind::IfElse { .. }
        | CommandKind::Switch { .. }
        | CommandKind::WhileLoop { .. }
        | CommandKind::ForLoop { .. }
        | CommandKind::Foreach { .. }
        | CommandKind::Catch { .. }
        | CommandKind::DictUpdate { .. } => true,
        _ => false,
    }
}

#[derive(Clone)]
pub enum CommandKind {
    Literal {
        value: SourceRef,
    },
    ProcCall {
        args: Vec<SourceRef>,
    },
    ArrayRef {
        args: Vec<SourceRef>,
    },
    Variable {
        value: SourceRef,
    },
    Set {
        var_name: SourceRef,
        value: SourceRef,
    },
    Op {
        args: Vec<SourceRef>,
        op: Cow<'static, str>,
    },
    ExprFunc {
        name: Cow<'static, str>,
        args: Vec<SourceRef>,
    },
    Expr {
        expr: SourceRef,
    },
    StrCat {
        args: Vec<SourceRef>,
    },
    ExpandedArg {
        value: SourceRef,
    },
    Catch {
        body: CmdBody,
    },
    Foreach {
        body: CmdBody,
        lists: Vec<Vec<SourceRef>>,
        values: Vec<SourceRef>,
    },
    DictUpdate {
        dict_var: SourceRef,
        pairs: Vec<(SourceRef, SourceRef)>,
        body: CmdBody,
    },
    Switch {
        value: SourceRef,
        arms: Vec<(Vec<SourceRef>, CmdBody)>,
        default: Option<CmdBody>,
        nocase: bool,
        integer: bool,
    },
    IfElse {
        test: CodeRef,
        if_body: CmdBody,
        else_body: CmdBody,
    },
    WhileLoop {
        body: CmdBody,
        condition: SourceRef,
    },
    ForLoop {
        body: CmdBody,
        setter: CodeRef,
        condition: CodeRef,
        incr: CodeRef,
    },
    Jump {
        info: CmdJump,
    },
}

#[derive(Clone)]
pub struct Command {
    pub base: CommandBase,
    pub kind: CommandKind,
}

impl Command {
    pub fn new(kind: CommandKind, children: Vec<CodeRef>) -> Self {
        Command {
            base: CommandBase::from_children(children),
            kind,
        }
    }

    pub fn merge(&mut self, codes: Vec<CodeRef>) {
        self.base.merge(codes);
    }

    pub fn merge_slice(&mut self, codes: &[CodeRef]) {
        self.base.merge_slice(codes);
    }

    pub fn merge_one(&mut self, code: CodeRef) {
        self.base.merge_one(code);
    }

    pub fn destack(&mut self) {
        self.base.destack();
        if let CommandKind::IfElse {
            if_body, else_body, ..
        } = &mut self.kind
        {
            if let Some((_, last)) = if_body.block.children.iter().next_back()
                && let Code::Command(cmd) = &mut *last.borrow_mut()
            {
                cmd.destack();
            }
            if let Some((_, last)) = else_body.block.children.iter().next_back()
                && let Code::Command(cmd) = &mut *last.borrow_mut()
            {
                cmd.destack();
            }
        }
    }

    pub fn write_source_repr(&self, out: &mut String) {
        use crate::command_render::*;
        match &self.kind {
            CommandKind::Literal { value } => {
                render_literal(out, value, self.base.stackn);
            }
            CommandKind::ProcCall { args } => {
                render_proc_call(out, args, self.base.stackn);
            }
            CommandKind::ArrayRef { args } => {
                render_array_ref(out, args);
            }
            CommandKind::Variable { value } => {
                render_variable(out, value);
            }
            CommandKind::Set { var_name, value } => {
                render_set(out, var_name, value, self.base.stackn);
            }
            CommandKind::Op { args, op } => {
                render_op(out, op, args, self.base.stackn);
            }
            CommandKind::ExprFunc { name, args } => {
                render_expr_func(out, name, args, self.base.stackn);
            }
            CommandKind::Expr { expr } => {
                render_expr(out, expr, self.base.stackn);
            }
            CommandKind::StrCat { args } => {
                render_strcat(out, args);
            }
            CommandKind::ExpandedArg { value } => {
                render_expanded_arg(out, value);
            }
            CommandKind::Catch { body } => {
                render_catch(out, body, self.base.stackn);
            }
            CommandKind::Foreach {
                body,
                lists,
                values,
            } => {
                render_foreach(out, body, lists, values);
            }
            CommandKind::DictUpdate {
                dict_var,
                pairs,
                body,
            } => {
                render_dict_update(out, dict_var, pairs, body);
            }
            CommandKind::Switch {
                value,
                arms,
                default,
                nocase,
                integer,
            } => {
                render_switch(out, value, arms, default.as_ref(), *nocase, *integer);
            }
            CommandKind::IfElse {
                test,
                if_body,
                else_body,
            } => {
                render_if_else(out, test.clone(), if_body, else_body);
            }
            CommandKind::WhileLoop { body, condition } => {
                render_while_loop(out, body, condition);
            }
            CommandKind::ForLoop {
                body,
                setter,
                condition,
                incr,
            } => {
                render_for_loop(out, body, setter.clone(), condition.clone(), incr.clone());
            }
            CommandKind::Jump { info } => {
                render_jump(out, info);
            }
        }
    }

    pub fn source_repr(&self) -> String {
        let mut out = String::new();
        self.write_source_repr(&mut out);
        out
    }

    pub fn write_expr_repr(&self, out: &mut String) {
        use crate::command_render::*;
        match &self.kind {
            CommandKind::Op { args, op } => write_op_expr(out, op, args),
            CommandKind::ExprFunc { name, args } => write_expr_func(out, name, args),
            CommandKind::Expr { .. } => self.write_source_repr(out),
            CommandKind::ProcCall { args } => write_expr_proc_call(out, args, self.base.stackn),
            _ => self.write_source_repr(out),
        }
    }

    pub fn expr_repr(&self) -> String {
        let mut out = String::new();
        self.write_expr_repr(&mut out);
        out
    }

    pub fn debug_description(&self) -> String {
        let mut output = self.source_repr();
        output.retain(|c| c != '\n' && c != '\r' && c != '\t');
        format!(
            "{}(\"{}\")",
            self.base_debug(),
            truncate(&output, 30, "...")
        )
    }

    fn base_debug(&self) -> String {
        let loc_str = right_justified(&format!("({}) ", self.base.block.location), 9, false);
        format!("{}Command", loc_str)
    }
}

#[derive(Clone)]
pub enum Code {
    Instruction(Instruction),
    Command(Command),
    Body(CmdBody),
}

impl Code {
    pub fn location(&self) -> i32 {
        match self {
            Code::Instruction(inst) => inst.location,
            Code::Command(cmd) => cmd.base.block.location,
            Code::Body(body) => body.block.location,
        }
    }

    pub fn size(&self) -> i32 {
        match self {
            Code::Instruction(inst) => inst.size(),
            Code::Command(cmd) => cmd.base.block.size(),
            Code::Body(body) => body.block.size(),
        }
    }

    pub fn range(&self) -> Range<i32> {
        let start = self.location();
        start..(start + self.size())
    }

    pub fn is_final(&self) -> bool {
        matches!(self, Code::Command(_))
    }

    pub fn for_each_next_location<F>(&self, f: &mut F)
    where
        F: FnMut(i32),
    {
        match self {
            Code::Instruction(inst) => {
                if let Some(target) = inst.jump_target() {
                    f(target);
                }
                if let Some(fallthrough) = inst.fallthrough_location() {
                    f(fallthrough);
                }
            }
            Code::Command(cmd) => cmd.base.block.for_each_next_location(f),
            Code::Body(body) => body.block.for_each_next_location(f),
        }
    }

    pub fn next_locations(&self) -> Vec<i32> {
        let mut out = Vec::with_capacity(2);
        self.for_each_next_location(&mut |loc| out.push(loc));
        out
    }

    pub fn source_repr(&self) -> String {
        let mut out = String::new();
        self.write_source_repr(&mut out);
        out
    }

    pub fn write_source_repr(&self, out: &mut String) {
        match self {
            Code::Instruction(_) => {}
            Code::Command(cmd) => cmd.write_source_repr(out),
            Code::Body(body) => body.write_source_repr(out),
        }
    }

    pub fn expr_repr(&self) -> String {
        let mut out = String::new();
        self.write_expr_repr(&mut out);
        out
    }

    pub fn write_expr_repr(&self, out: &mut String) {
        match self {
            Code::Instruction(_) => {}
            Code::Command(cmd) => cmd.write_expr_repr(out),
            Code::Body(body) => body.write_source_repr(out),
        }
    }

    pub fn debug_description(&self) -> String {
        match self {
            Code::Instruction(inst) => inst.debug_description(),
            Code::Command(cmd) => cmd.debug_description(),
            Code::Body(body) => {
                let output = truncate(&body.source_repr().replace('\n', " "), 20, "...");
                format!("CmdBody(\"{}\")", output)
            }
        }
    }

    pub fn is_command(&self) -> bool {
        matches!(self, Code::Command(_))
    }

    pub fn as_instruction(&self) -> Option<&Instruction> {
        if let Code::Instruction(inst) = self {
            Some(inst)
        } else {
            None
        }
    }

    pub fn as_command_mut(&mut self) -> Option<&mut Command> {
        if let Code::Command(cmd) = self {
            Some(cmd)
        } else {
            None
        }
    }
}

impl SourceRepresentable for Code {
    fn source_repr(&self) -> String {
        Code::source_repr(self)
    }

    fn write_source_repr(&self, out: &mut String) {
        Code::write_source_repr(self, out);
    }
}

pub fn code_ref(code: Code) -> CodeRef {
    let node = Rc::new(RefCell::new(code));
    CODE_NODES.with(|nodes| nodes.borrow_mut().push(Rc::downgrade(&node)));
    CodeRef(node)
}

pub fn cmd_literal(children: Vec<CodeRef>, value: SourceRef) -> CodeRef {
    code_ref(Code::Command(Command::new(
        CommandKind::Literal { value },
        children,
    )))
}

pub fn cmd_proc_call(children: Vec<CodeRef>, args: Vec<SourceRef>) -> CodeRef {
    code_ref(Code::Command(Command::new(
        CommandKind::ProcCall { args },
        children,
    )))
}

pub fn cmd_array_ref(children: Vec<CodeRef>, args: Vec<SourceRef>) -> CodeRef {
    code_ref(Code::Command(Command::new(
        CommandKind::ArrayRef { args },
        children,
    )))
}

pub fn cmd_variable(children: Vec<CodeRef>, value: SourceRef) -> CodeRef {
    code_ref(Code::Command(Command::new(
        CommandKind::Variable { value },
        children,
    )))
}

pub fn cmd_set(children: Vec<CodeRef>, var_name: SourceRef, value: SourceRef) -> CodeRef {
    code_ref(Code::Command(Command::new(
        CommandKind::Set { var_name, value },
        children,
    )))
}

pub fn cmd_op(
    children: Vec<CodeRef>,
    op: impl Into<Cow<'static, str>>,
    args: Vec<SourceRef>,
) -> CodeRef {
    code_ref(Code::Command(Command::new(
        CommandKind::Op {
            args,
            op: op.into(),
        },
        children,
    )))
}

pub fn cmd_expr_func(
    children: Vec<CodeRef>,
    name: impl Into<Cow<'static, str>>,
    args: Vec<SourceRef>,
) -> CodeRef {
    code_ref(Code::Command(Command::new(
        CommandKind::ExprFunc {
            name: name.into(),
            args,
        },
        children,
    )))
}

pub fn cmd_expr(children: Vec<CodeRef>, expr: SourceRef) -> CodeRef {
    code_ref(Code::Command(Command::new(
        CommandKind::Expr { expr },
        children,
    )))
}

pub fn cmd_strcat(children: Vec<CodeRef>, args: Vec<SourceRef>) -> CodeRef {
    code_ref(Code::Command(Command::new(
        CommandKind::StrCat { args },
        children,
    )))
}

pub fn cmd_expanded_arg(children: Vec<CodeRef>, value: SourceRef) -> CodeRef {
    code_ref(Code::Command(Command::new(
        CommandKind::ExpandedArg { value },
        children,
    )))
}

pub fn cmd_catch(children: Vec<CodeRef>, body: CmdBody) -> CodeRef {
    code_ref(Code::Command(Command::new(
        CommandKind::Catch { body },
        children,
    )))
}

pub fn cmd_foreach(
    children: Vec<CodeRef>,
    body: CmdBody,
    lists: Vec<Vec<SourceRef>>,
    values: Vec<SourceRef>,
) -> CodeRef {
    code_ref(Code::Command(Command::new(
        CommandKind::Foreach {
            body,
            lists,
            values,
        },
        children,
    )))
}

pub fn cmd_if_else(
    children: Vec<CodeRef>,
    test: CodeRef,
    if_body: CmdBody,
    else_body: CmdBody,
) -> CodeRef {
    code_ref(Code::Command(Command::new(
        CommandKind::IfElse {
            test,
            if_body,
            else_body,
        },
        children,
    )))
}

pub fn cmd_while_loop(children: Vec<CodeRef>, body: CmdBody, condition: SourceRef) -> CodeRef {
    code_ref(Code::Command(Command::new(
        CommandKind::WhileLoop { body, condition },
        children,
    )))
}

pub fn cmd_for_loop(
    children: Vec<CodeRef>,
    body: CmdBody,
    setter: CodeRef,
    condition: CodeRef,
    incr: CodeRef,
) -> CodeRef {
    code_ref(Code::Command(Command::new(
        CommandKind::ForLoop {
            body,
            setter,
            condition,
            incr,
        },
        children,
    )))
}

pub fn cmd_dict_update(
    children: Vec<CodeRef>,
    dict_var: SourceRef,
    pairs: Vec<(SourceRef, SourceRef)>,
    body: CmdBody,
) -> CodeRef {
    code_ref(Code::Command(Command::new(
        CommandKind::DictUpdate {
            dict_var,
            pairs,
            body,
        },
        children,
    )))
}

pub fn cmd_switch(
    children: Vec<CodeRef>,
    value: SourceRef,
    arms: Vec<(Vec<SourceRef>, CmdBody)>,
    default: Option<CmdBody>,
    nocase: bool,
    integer: bool,
) -> CodeRef {
    code_ref(Code::Command(Command::new(
        CommandKind::Switch {
            value,
            arms,
            default,
            nocase,
            integer,
        },
        children,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::Instruction;
    use crate::source::sr;
    use tcld_core::inst_table::DEFAULT_INSTRUCTION_SET;

    fn dummy_inst(location: i32) -> CodeRef {
        let opcode = DEFAULT_INSTRUCTION_SET
            .opcode_from_i32(0)
            .expect("opcode exists");
        let desc = *DEFAULT_INSTRUCTION_SET
            .desc_from_i32(0)
            .expect("instruction desc exists");
        code_ref(Code::Instruction(Instruction::new(
            location,
            opcode,
            desc,
            vec![],
            None,
        )))
    }

    fn set_cmd(location: i32, name: &str, value: &str) -> CodeRef {
        let code = cmd_set(
            vec![dummy_inst(location)],
            sr(name.to_string()),
            sr(value.to_string()),
        );
        if let Code::Command(cmd) = &mut *code.borrow_mut() {
            cmd.destack();
        }
        code
    }

    fn literal_cmd(location: i32, value: &str) -> CodeRef {
        cmd_literal(vec![dummy_inst(location)], sr(value.to_string()))
    }

    #[test]
    fn cmd_body_inserts_blank_line_after_proc_definitions() {
        let proc_cmd = cmd_proc_call(
            vec![dummy_inst(10)],
            vec![sr("proc"), sr("alpha"), sr("{}"), sr("set a 1")],
        );
        if let Code::Command(cmd) = &mut *proc_cmd.borrow_mut() {
            cmd.destack();
        }
        let tail = set_cmd(20, "b", "2");
        let body = CmdBody::new(vec![proc_cmd, tail]);

        assert_eq!(body.source_repr(), "proc alpha {} {set a 1}\n\nset b 2");
    }

    #[test]
    fn cmd_body_inserts_blank_line_after_if_blocks() {
        let test = literal_cmd(11, "1");
        let if_body = CmdBody::new(vec![set_cmd(12, "a", "1")]);
        let else_body = CmdBody::new(vec![]);
        let if_cmd = cmd_if_else(vec![dummy_inst(10)], test, if_body, else_body);
        if let Code::Command(cmd) = &mut *if_cmd.borrow_mut() {
            cmd.destack();
        }
        let tail = set_cmd(20, "b", "2");
        let body = CmdBody::new(vec![if_cmd, tail]);

        assert_eq!(body.source_repr(), "if 1 {\n    set a 1\n}\n\nset b 2");
    }

    #[test]
    fn cmd_body_keeps_single_newline_between_simple_commands() {
        let one = set_cmd(10, "a", "1");
        let two = set_cmd(20, "b", "2");
        let body = CmdBody::new(vec![one, two]);

        assert_eq!(body.source_repr(), "set a 1\nset b 2");
    }

    #[test]
    fn code_nodes_are_reclaimed_without_invalidating_live_refs() {
        reset_code_arena();
        let baseline = code_arena_len();
        let code = literal_cmd(40, "live");
        assert_eq!(code_arena_len(), baseline + 2);
        reset_code_arena();
        assert_eq!(code.borrow().source_repr(), "live");
        drop(code);
        reset_code_arena();
        assert_eq!(code_arena_len(), baseline);
    }
}
