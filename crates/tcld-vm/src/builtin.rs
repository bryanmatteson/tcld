//! VarNameBuiltin handlers (S05).

use crate::error::VmError;
use crate::value::{
    ByteString, VmObj, list_to_string, parse_array_name, parse_tcl_int, split_tcl_list,
};
use crate::var::{VarLink, split_var_name};

/// Context the engine supplies to builtins so they can touch locals / host.
pub trait VarNameCtx {
    fn get_var(&mut self, name1: &ByteString, name2: Option<&ByteString>)
    -> Result<VmObj, VmError>;
    fn set_var(
        &mut self,
        name1: &ByteString,
        name2: Option<&ByteString>,
        value: VmObj,
    ) -> Result<(), VmError>;
    fn unset_var(&mut self, name1: &ByteString, name2: Option<&ByteString>) -> Result<(), VmError>;
    fn install_host_link(
        &mut self,
        local_name: &ByteString,
        name1: ByteString,
        name2: Option<ByteString>,
    ) -> Result<(), VmError>;
    fn current_ns(&self) -> &ByteString;
}

/// Run a VarNameBuiltin. `cmd` is bare name (`set`, `append`, …); `args` exclude cmd word.
pub fn run_var_name_builtin(
    cmd: &[u8],
    args: &[VmObj],
    ctx: &mut dyn VarNameCtx,
) -> Result<VmObj, VmError> {
    match cmd {
        b"set" => builtin_set(args, ctx),
        b"append" => builtin_append(args, ctx),
        b"lappend" => builtin_lappend(args, ctx),
        b"incr" => builtin_incr(args, ctx),
        b"unset" => builtin_unset(args, ctx),
        b"global" => builtin_global(args, ctx),
        b"variable" => builtin_variable(args, ctx),
        _ => Err(VmError::Internal(format!(
            "not a VarNameBuiltin: {}",
            String::from_utf8_lossy(cmd)
        ))),
    }
}

fn arg_bytes(args: &[VmObj], i: usize, cmd: &str) -> Result<ByteString, VmError> {
    args.get(i)
        .ok_or_else(|| VmError::VarNameUnsupported {
            cmd: cmd.into(),
            form: "missing argument".into(),
        })?
        .ensure_bytes()
}

fn builtin_set(args: &[VmObj], ctx: &mut dyn VarNameCtx) -> Result<VmObj, VmError> {
    match args.len() {
        1 => {
            let name = arg_bytes(args, 0, "set")?;
            let (n1, n2) = split_var_name(&name);
            ctx.get_var(&n1, n2.as_ref())
        }
        2 => {
            let name = arg_bytes(args, 0, "set")?;
            let (n1, n2) = split_var_name(&name);
            let val = args[1].clone();
            ctx.set_var(&n1, n2.as_ref(), val.clone())?;
            Ok(val)
        }
        _ => Err(VmError::VarNameUnsupported {
            cmd: "set".into(),
            form: format!("arity {}", args.len()),
        }),
    }
}

fn builtin_append(args: &[VmObj], ctx: &mut dyn VarNameCtx) -> Result<VmObj, VmError> {
    if args.is_empty() {
        return Err(VmError::VarNameUnsupported {
            cmd: "append".into(),
            form: "no args".into(),
        });
    }
    let name = arg_bytes(args, 0, "append")?;
    let (n1, n2) = split_var_name(&name);
    let mut base = match ctx.get_var(&n1, n2.as_ref()) {
        Ok(v) => v.ensure_bytes()?.as_bytes().to_vec(),
        Err(error) if is_missing_value(&error) => Vec::new(),
        Err(error) => return Err(error),
    };
    for a in &args[1..] {
        base.extend_from_slice(a.ensure_bytes()?.as_bytes());
    }
    let result = VmObj::from_bytes(base);
    ctx.set_var(&n1, n2.as_ref(), result.clone())?;
    Ok(result)
}

fn builtin_lappend(args: &[VmObj], ctx: &mut dyn VarNameCtx) -> Result<VmObj, VmError> {
    if args.is_empty() {
        return Err(VmError::VarNameUnsupported {
            cmd: "lappend".into(),
            form: "no args".into(),
        });
    }
    let name = arg_bytes(args, 0, "lappend")?;
    let (n1, n2) = split_var_name(&name);
    let mut list = match ctx.get_var(&n1, n2.as_ref()) {
        Ok(v) => v.as_list_vec()?,
        Err(error) if is_missing_value(&error) => Vec::new(),
        Err(error) => return Err(error),
    };
    for a in &args[1..] {
        list.push(a.clone());
    }
    let s = list_to_string(&list)?;
    let result = VmObj::from_str(&s);
    ctx.set_var(&n1, n2.as_ref(), result.clone())?;
    Ok(result)
}

fn builtin_incr(args: &[VmObj], ctx: &mut dyn VarNameCtx) -> Result<VmObj, VmError> {
    if args.is_empty() || args.len() > 2 {
        return Err(VmError::VarNameUnsupported {
            cmd: "incr".into(),
            form: format!("arity {}", args.len()),
        });
    }
    let name = arg_bytes(args, 0, "incr")?;
    let (n1, n2) = split_var_name(&name);
    let delta = if args.len() == 2 {
        args[1].as_i64()?
    } else {
        1
    };
    let cur = match ctx.get_var(&n1, n2.as_ref()) {
        Ok(v) => v.as_i64()?,
        Err(error) if is_missing_value(&error) => 0,
        Err(error) => return Err(error),
    };
    let next = VmObj::Int(cur + delta);
    ctx.set_var(&n1, n2.as_ref(), next.clone())?;
    Ok(next)
}

fn is_missing_value(error: &VmError) -> bool {
    matches!(
        error,
        VmError::TclError { message, .. }
            if message.contains("no such variable") || message.contains("no such element in array")
    )
}

fn builtin_unset(args: &[VmObj], ctx: &mut dyn VarNameCtx) -> Result<VmObj, VmError> {
    // Support: unset ?-nocomplain? name ...
    let mut nocomplain = false;
    let mut i = 0;
    if let Some(first) = args.first() {
        let b = first.ensure_bytes()?;
        if b.as_bytes() == b"-nocomplain" {
            nocomplain = true;
            i = 1;
        }
    }
    if i >= args.len() {
        return Err(VmError::VarNameUnsupported {
            cmd: "unset".into(),
            form: "no names".into(),
        });
    }
    for a in &args[i..] {
        let name = a.ensure_bytes()?;
        let (n1, n2) = split_var_name(&name);
        match ctx.unset_var(&n1, n2.as_ref()) {
            Ok(()) => {}
            Err(_e) if nocomplain => {}
            Err(e) => return Err(e),
        }
    }
    Ok(VmObj::empty())
}

fn builtin_global(args: &[VmObj], ctx: &mut dyn VarNameCtx) -> Result<VmObj, VmError> {
    if args.is_empty() {
        return Err(VmError::VarNameUnsupported {
            cmd: "global".into(),
            form: "no names".into(),
        });
    }
    for a in args {
        let name = a.ensure_bytes()?;
        let (simple, part2) = parse_array_name(name.as_bytes());
        if part2.is_some() {
            return Err(VmError::VarNameUnsupported {
                cmd: "global".into(),
                form: "array element".into(),
            });
        }
        // fully qualify global
        let host_name = if simple.as_bytes().starts_with(b"::") {
            simple.clone()
        } else {
            ByteString::from(format!("::{}", simple.as_str_lossy()).as_str())
        };
        // local name is the leaf
        let leaf = simple
            .as_bytes()
            .rsplit(|b| *b == b':')
            .next()
            .map(ByteString::from_bytes)
            .unwrap_or(simple.clone());
        ctx.install_host_link(&leaf, host_name, None)?;
    }
    Ok(VmObj::empty())
}

fn builtin_variable(args: &[VmObj], ctx: &mut dyn VarNameCtx) -> Result<VmObj, VmError> {
    match args.len() {
        1 => {
            let name = arg_bytes(args, 0, "variable")?;
            install_variable_link(ctx, &name, None)?;
            Ok(VmObj::empty())
        }
        2 => {
            let name = arg_bytes(args, 0, "variable")?;
            let val = args[1].clone();
            install_variable_link(ctx, &name, Some(val))?;
            Ok(VmObj::empty())
        }
        _ => Err(VmError::VarNameUnsupported {
            cmd: "variable".into(),
            form: format!("arity {}", args.len()),
        }),
    }
}

fn install_variable_link(
    ctx: &mut dyn VarNameCtx,
    name: &ByteString,
    value: Option<VmObj>,
) -> Result<(), VmError> {
    let (n1, n2) = parse_array_name(name.as_bytes());
    let host_name = qualify_ns_name(ctx.current_ns(), &n1);
    let leaf = n1
        .as_bytes()
        .rsplit(|b| *b == b':')
        .next()
        .map(ByteString::from_bytes)
        .unwrap_or(n1.clone());
    ctx.install_host_link(&leaf, host_name, n2.clone())?;
    if let Some(v) = value {
        ctx.set_var(&leaf, n2.as_ref(), v)?;
    }
    Ok(())
}

fn qualify_ns_name(current_ns: &ByteString, name: &ByteString) -> ByteString {
    if name.as_bytes().starts_with(b"::") {
        return name.clone();
    }
    let ns = current_ns.as_str_lossy();
    let ns = ns.trim_start_matches(':');
    if ns.is_empty() {
        ByteString::from(format!("::{}", name.as_str_lossy()).as_str())
    } else {
        ByteString::from(format!("::{ns}::{}", name.as_str_lossy()).as_str())
    }
}

// Keep unused import quiet if split_tcl_list unused in some builds.
#[allow(dead_code)]
fn _use_split(b: &[u8]) -> Result<Vec<VmObj>, VmError> {
    split_tcl_list(b)
}

#[allow(dead_code)]
fn _use_parse(s: &str) -> Option<i64> {
    parse_tcl_int(s)
}

#[allow(dead_code)]
fn _use_link() -> VarLink {
    VarLink::HostVar {
        name1: ByteString::from("x"),
        name2: None,
    }
}
