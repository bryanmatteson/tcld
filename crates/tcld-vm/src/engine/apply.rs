//! Anonymous procedures execute their exact body in a mapped, fresh Tcl scope.
use super::*;

struct Formal {
    name: ByteString,
    default: Option<VmObj>,
}

impl HybridEngineInner {
    pub(super) fn invoke_apply(
        &mut self,
        words: &[VmObj],
        site: CodeAddress,
    ) -> Result<InvokeOutcome, VmError> {
        let invoked = words[0].as_string()?;
        let Some(lambda) = words.get(1) else {
            return Err(crate::args::wrong_args(&[&invoked], "lambdaExpr ?arg ...?"));
        };
        let lambda_text = lambda.as_string()?;
        let parts = lambda
            .as_list_vec()
            .ok()
            .filter(|parts| matches!(parts.len(), 2 | 3))
            .ok_or_else(|| {
                crate::args::tcl_error(
                    format!("can't interpret \"{lambda_text}\" as a lambda expression"),
                    &["TCL", "VALUE", "LAMBDA"],
                )
            })?;
        let formals = parse_formals(&parts[0]).map_err(|error| match error {
            VmError::TclError {
                message,
                error_info,
                error_code,
            } => VmError::TclError {
                error_info: Some(format!(
                    "{}\n    (parsing lambda expression \"{lambda_text}\")",
                    error_info.as_deref().unwrap_or(&message)
                )),
                message,
                error_code,
            },
            other => other,
        })?;

        // Lambda namespace names are always global references, even when the
        // caller is inside another namespace. Tcl validates this before arity.
        let requested = parts
            .get(2)
            .map(VmObj::ensure_bytes)
            .transpose()?
            .unwrap_or_else(|| ByteString::from("::"));
        let requested = NamespaceName::root().qualify(&requested);
        let namespace = crate::world::canonical(&requested);
        let exists = if self.mode == RuntimeMode::VmStrict {
            self.world.namespace_exists(&namespace)
        } else {
            self.generated_leaf(&[
                VmObj::from_str("namespace"),
                VmObj::from_str("exists"),
                VmObj::Bytes(namespace.clone()),
            ])?
            .as_string()?
                != "0"
        };
        if !exists {
            let name = requested.as_str_lossy();
            return Err(crate::args::tcl_error(
                format!("namespace \"{name}\" not found"),
                &["TCL", "LOOKUP", "NAMESPACE", &name],
            ));
        }

        let actual = &words[2..];
        let variadic = formals
            .last()
            .is_some_and(|formal| formal.name.as_bytes() == b"args");
        let fixed = formals.len() - usize::from(variadic);
        if (!variadic && actual.len() > fixed)
            || formals[..fixed]
                .iter()
                .skip(actual.len())
                .any(|formal| formal.default.is_none())
        {
            return Err(lambda_wrong_args(&invoked, &formals)?);
        }
        let mut bindings = Vec::with_capacity(formals.len());
        for (index, formal) in formals.iter().enumerate() {
            // Tcl keeps separate argument slots for duplicate names, but body
            // variable resolution always chooses their first occurrence.
            if bindings
                .iter()
                .any(|(name, _): &(ByteString, VmObj)| name == &formal.name)
            {
                continue;
            }
            let value = if variadic && index == fixed {
                VmObj::List(actual.get(fixed..).unwrap_or_default().to_vec())
            } else {
                actual
                    .get(index)
                    .cloned()
                    .or_else(|| formal.default.clone())
                    .expect("validated lambda argument")
            };
            bindings.push((formal.name.clone(), value));
        }
        self.enter_generated(
            "apply-body",
            parts[1].as_string()?,
            site,
            NamespaceName(namespace),
            None,
            FrameKind::Lambda,
            None,
        )?;
        self.frames.last_mut().unwrap().invocation = Some(VmObj::List(words.to_vec()));
        for (name, value) in bindings {
            self.var_set(&name, None, value)?;
        }
        Ok(InvokeOutcome::FramePushed)
    }

    pub(super) fn lambda_error_context(
        &self,
        completion: &mut Completion,
        producing_pc: u32,
    ) -> Result<(), VmError> {
        let frame = &self.frames[self.fi()];
        if completion.code == 1 {
            let invocation = frame.invocation.as_ref().unwrap().as_list_vec()?;
            let lambda = invocation[1].as_string()?;
            let mut end = lambda.len().min(60);
            while !lambda.is_char_boundary(end) {
                end -= 1;
            }
            let line = self.mapped_error_line(producing_pc);
            completion.append_error_context(&format!(
                "\n    (lambda term \"{}{}\" line {line})",
                &lambda[..end],
                if lambda.len() > end { "..." } else { "" },
            ))?;
        }
        Ok(())
    }
}

fn parse_formals(spec: &VmObj) -> Result<Vec<Formal>, VmError> {
    let format_error = |message| {
        crate::args::tcl_error(
            message,
            &["TCL", "OPERATION", "PROC", "FORMALARGUMENTFORMAT"],
        )
    };
    spec.as_list_vec()?
        .iter()
        .map(|specifier| {
            let fields = specifier.as_list_vec()?;
            if fields.len() > 2 {
                return Err(format_error(format!(
                    "too many fields in argument specifier \"{}\"",
                    specifier.as_string()?
                )));
            }
            let Some(name) = fields.first() else {
                return Err(format_error("argument with no name".into()));
            };
            let name = name.ensure_bytes()?;
            let bytes = name.as_bytes();
            if bytes.is_empty() {
                return Err(format_error("argument with no name".into()));
            }
            for (index, byte) in bytes[..bytes.len() - 1].iter().enumerate() {
                if *byte == b'(' && bytes.last() == Some(&b')') {
                    return Err(format_error(format!(
                        "formal parameter \"{}\" is an array element",
                        name.as_str_lossy()
                    )));
                }
                if *byte == b':' && bytes[index + 1] == b':' {
                    return Err(format_error(format!(
                        "formal parameter \"{}\" is not a simple name",
                        name.as_str_lossy()
                    )));
                }
            }
            Ok(Formal {
                name,
                default: fields.get(1).cloned(),
            })
        })
        .collect()
}

fn lambda_wrong_args(invoked: &str, formals: &[Formal]) -> Result<VmError, VmError> {
    let mut words = vec![VmObj::from_str(invoked), VmObj::from_str("lambdaExpr")];
    let mut rest = false;
    for (index, formal) in formals.iter().enumerate() {
        let name = formal.name.as_str_lossy();
        if formal.default.is_some() {
            words.push(VmObj::from_str(&format!("?{name}?")));
        } else if index + 1 == formals.len() && name == "args" {
            rest = true;
        } else {
            words.push(VmObj::Bytes(formal.name.clone()));
        }
    }
    Ok(crate::args::tcl_error(
        format!(
            "wrong # args: should be \"{}{}\"",
            list_to_string(&words)?,
            if rest { " ?arg ...?" } else { "" }
        ),
        &["TCL", "WRONGARGS"],
    ))
}
