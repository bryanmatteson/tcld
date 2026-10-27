//! Tcl completion records shared by VM, host-value, and host-frame paths.

use crate::error::VmError;
use crate::value::{ByteString, VmObj};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ReturnOptions {
    pub level: u32,
    pub code: i32,
    pub error_info: Option<ByteString>,
    pub error_code: Option<VmObj>,
    pub raw: Option<VmObj>,
}

impl ReturnOptions {
    /// Tcl's return-option expansion and validation, including nested
    /// `-options` dictionaries and application-defined keys.
    pub(crate) fn from_return_options(raw: VmObj) -> Result<Self, VmError> {
        let mut expanded = crate::dict::Dict::default();
        let words = raw
            .as_list_vec()
            .ok()
            .filter(|words| words.len().is_multiple_of(2))
            .ok_or_else(|| {
                crate::args::tcl_error(
                    format!(
                        "expected dict but got \"{}\"",
                        raw.as_string().unwrap_or_default()
                    ),
                    &["TCL", "RESULT", "ILLEGAL_OPTIONS"],
                )
            })?;
        let mut stack = vec![words.into_iter()];
        while let Some(words) = stack.last_mut() {
            let Some(key) = words.next() else {
                stack.pop();
                continue;
            };
            let value = words.next().ok_or_else(|| {
                crate::args::tcl_error(
                    "return options must have an even number of elements",
                    &["TCL", "RESULT", "ILLEGAL_OPTIONS"],
                )
            })?;
            if key.ensure_bytes()?.as_bytes() == b"-options" {
                let nested = value
                    .as_list_vec()
                    .ok()
                    .filter(|words| words.len().is_multiple_of(2))
                    .ok_or_else(|| {
                        crate::args::tcl_error(
                            format!(
                                "bad -options value: expected dictionary but got \"{}\"",
                                value.as_string().unwrap_or_default()
                            ),
                            &["TCL", "RESULT", "ILLEGAL_OPTIONS"],
                        )
                    })?;
                stack.push(nested.into_iter());
            } else {
                expanded.put(key, value)?;
            }
        }
        let get = |key| expanded.get(&VmObj::from_str(key));
        let mut code = get("-code")?
            .map(parse_completion_code)
            .transpose()?
            .unwrap_or(0);
        let mut level = match get("-level")? {
            Some(value) => tcl_int(value).filter(|level| *level >= 0).ok_or_else(|| {
                crate::args::tcl_error(
                    format!(
                        "bad -level value: expected non-negative integer but got \"{}\"",
                        value.as_string().unwrap_or_default()
                    ),
                    &["TCL", "RESULT", "ILLEGAL_LEVEL"],
                )
            })? as u32,
            None => 1,
        };
        for (key, bad_list, bad_length) in [
            ("-errorcode", "ILLEGAL_ERRORCODE", None),
            (
                "-errorstack",
                "NONLIST_ERRORSTACK",
                Some("ODDSIZEDLIST_ERRORSTACK"),
            ),
        ] {
            if let Some(value) = get(key)? {
                let words = value.as_list_vec().map_err(|_| {
                    crate::args::tcl_error(
                        format!(
                            "bad {key} value: expected a list but got \"{}\"",
                            value.as_string().unwrap_or_default()
                        ),
                        &["TCL", "RESULT", bad_list],
                    )
                })?;
                if let Some(error) = bad_length.filter(|_| !words.len().is_multiple_of(2)) {
                    return Err(crate::args::tcl_error(
                        format!(
                            "forbidden odd-sized list for {key}: \"{}\"",
                            value.as_string()?
                        ),
                        &["TCL", "RESULT", error],
                    ));
                }
            }
        }
        let has_error_code = get("-errorcode")?.is_some();
        if code == 2 {
            code = 0;
            level += 1;
        }
        expanded.put(VmObj::from_str("-code"), VmObj::from_int(code as i64))?;
        expanded.put(VmObj::from_str("-level"), VmObj::from_int(level as i64))?;
        if code == 1 && !has_error_code {
            expanded.put(VmObj::from_str("-errorcode"), VmObj::from_str("NONE"))?;
        }
        Self::from_vm_obj(expanded.into_value())
    }

    pub(crate) fn sync_raw_code_level(&mut self) -> Result<(), VmError> {
        let mut raw = crate::dict::Dict::from_optional(self.raw.as_ref())?;
        raw.put(VmObj::from_str("-code"), VmObj::from_int(self.code as i64))?;
        raw.put(
            VmObj::from_str("-level"),
            VmObj::from_int(self.level as i64),
        )?;
        if self.code == 1 && self.error_code.is_none() {
            raw.put(VmObj::from_str("-errorcode"), VmObj::from_str("NONE"))?;
        }
        self.raw = Some(raw.into_value());
        Ok(())
    }

    pub fn for_code(code: i32) -> Self {
        Self {
            level: u32::from(code == 2),
            code,
            ..Self::default()
        }
    }

    pub fn as_vm_obj(&self) -> VmObj {
        if let Some(raw) = &self.raw {
            return raw.clone();
        }
        let mut words = vec![
            VmObj::from_str("-code"),
            VmObj::from_int(self.code as i64),
            VmObj::from_str("-level"),
            VmObj::from_int(self.level as i64),
        ];
        if let Some(info) = &self.error_info {
            words.push(VmObj::from_str("-errorinfo"));
            words.push(VmObj::Bytes(info.clone()));
        }
        // Tcl gives every error an -errorcode, NONE when nothing set one.
        let error_code = match &self.error_code {
            Some(code) => Some(code.clone()),
            None if self.code == 1 => Some(VmObj::from_str("NONE")),
            None => None,
        };
        if let Some(code) = error_code {
            words.push(VmObj::from_str("-errorcode"));
            words.push(code);
        }
        VmObj::List(words)
    }

    pub fn from_vm_obj(raw: VmObj) -> Result<Self, VmError> {
        let words = raw.as_list_vec()?;
        if words.len() % 2 != 0 {
            return Err(VmError::TclError {
                message: "return options must have an even number of elements".into(),
                error_info: None,
                error_code: Some("TCL RESULT ILLEGAL_OPTIONS".into()),
            });
        }
        let mut options = Self {
            raw: Some(raw),
            ..Self::default()
        };
        for pair in words.chunks_exact(2) {
            let key = pair[0].ensure_bytes()?;
            match key.as_bytes() {
                b"-code" => options.code = parse_completion_code(&pair[1])?,
                b"-level" => {
                    let level = pair[1].as_i64()?;
                    if level < 0 {
                        return Err(VmError::TclError {
                            message: "bad -level value: expected non-negative integer".into(),
                            error_info: None,
                            error_code: Some("TCL RESULT ILLEGAL_LEVEL".into()),
                        });
                    }
                    options.level = level as u32;
                }
                b"-errorinfo" => options.error_info = Some(pair[1].ensure_bytes()?),
                b"-errorcode" => options.error_code = Some(pair[1].clone()),
                _ => {}
            }
        }
        Ok(options)
    }

    pub fn routed_code(&self) -> i32 {
        if self.level > 0 { 2 } else { self.code }
    }

    pub(crate) fn consume_return_level(&mut self) -> Result<(), VmError> {
        self.level -= 1;
        if self.raw.is_some() {
            self.sync_raw_code_level()?;
        }
        Ok(())
    }
}

pub(crate) fn parse_completion_code(value: &VmObj) -> Result<i32, VmError> {
    let bytes = value.ensure_bytes()?;
    let code = match bytes.as_bytes() {
        b"ok" => 0,
        b"error" => 1,
        b"return" => 2,
        b"break" => 3,
        b"continue" => 4,
        _ => return tcl_int(value).ok_or_else(|| crate::args::tcl_error(
            format!("bad completion code \"{}\": must be ok, error, return, break, continue, or an integer", bytes.as_str_lossy()),
            &["TCL", "RESULT", "ILLEGAL_CODE"],
        )),
    };
    Ok(code)
}

fn tcl_int(value: &VmObj) -> Option<i32> {
    crate::args::wide(value)
        .ok()
        .filter(|value| (i32::MIN as i64..=u32::MAX as i64).contains(value))
        .map(|value| value as i32)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Completion {
    pub code: i32,
    pub result: VmObj,
    pub options: ReturnOptions,
}

impl Completion {
    /// Tcl's procedure boundary replaces loop control with an error while
    /// retaining application return options. Error metadata belongs to the new
    /// error, even if a non-error `return` supplied similarly named options.
    pub(crate) fn reject_loop_control(&mut self) -> Result<(), VmError> {
        debug_assert!(matches!(self.code, 3 | 4));
        let command = if self.code == 3 { "break" } else { "continue" };
        let mut options = crate::dict::Dict::from_value(&self.options.as_vm_obj())?;
        for key in ["-errorinfo", "-errorstack", "-errorline"] {
            options.remove(&VmObj::from_str(key))?;
        }
        options.put(VmObj::from_str("-code"), VmObj::from_int(1))?;
        options.put(VmObj::from_str("-level"), VmObj::from_int(0))?;
        options.put(
            VmObj::from_str("-errorcode"),
            VmObj::from_str("TCL RESULT UNEXPECTED"),
        )?;
        self.code = 1;
        self.result = VmObj::from_str(&format!("invoked \"{command}\" outside of a loop"));
        options.put(VmObj::from_str("-errorinfo"), self.result.clone())?;
        self.options = ReturnOptions::from_vm_obj(options.into_value())?;
        Ok(())
    }

    pub(crate) fn from_return_args(args: &[VmObj]) -> Result<Self, VmError> {
        let explicit_result = !args.len().is_multiple_of(2);
        let option_end = args.len() - usize::from(explicit_result);
        let options = ReturnOptions::from_return_options(VmObj::List(args[..option_end].to_vec()))?;
        Ok(Self {
            code: options.routed_code(),
            result: if explicit_result {
                args.last().unwrap().clone()
            } else {
                VmObj::empty()
            },
            options,
        })
    }

    /// Add evaluation context without losing caller-supplied return options.
    pub(crate) fn append_error_context(&mut self, context: &str) -> Result<(), VmError> {
        let info = self
            .options
            .error_info
            .take()
            .unwrap_or(self.result.ensure_bytes()?);
        let mut info = info.as_bytes().to_vec();
        info.extend_from_slice(context.as_bytes());
        let info = ByteString::from_bytes(info);
        if let Some(raw) = self.options.raw.take() {
            let mut options = raw.as_list_vec()?;
            if let Some(pair) = options.chunks_exact_mut(2).find(|pair| {
                pair[0]
                    .ensure_bytes()
                    .is_ok_and(|key| key.as_bytes() == b"-errorinfo")
            }) {
                pair[1] = VmObj::Bytes(info.clone());
            } else {
                options.extend([VmObj::from_str("-errorinfo"), VmObj::Bytes(info.clone())]);
            }
            self.options.raw = Some(VmObj::List(options));
        }
        self.options.error_info = Some(info);
        Ok(())
    }

    pub fn ok(result: VmObj) -> Self {
        Self {
            code: 0,
            result,
            options: ReturnOptions::default(),
        }
    }

    pub fn with_code(code: i32, result: VmObj) -> Self {
        Self {
            code,
            result,
            options: ReturnOptions::for_code(code),
        }
    }

    pub fn error(result: VmObj, error_info: Option<ByteString>, error_code: Option<VmObj>) -> Self {
        Self {
            code: 1,
            result,
            options: ReturnOptions {
                code: 1,
                error_info,
                error_code,
                ..ReturnOptions::default()
            },
        }
    }

    pub fn returning(result: VmObj, level: u32, code: i32, raw: Option<VmObj>) -> Self {
        Self {
            code: 2,
            result,
            options: ReturnOptions {
                level,
                code,
                raw,
                ..ReturnOptions::default()
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn return_options_parse_named_code_and_preserve_raw() {
        let raw = VmObj::from_str("-code return -level 2 -errorinfo {trace text} -errorcode {A B}");
        let options = ReturnOptions::from_vm_obj(raw.clone()).expect("options");
        assert_eq!(options.code, 2);
        assert_eq!(options.level, 2);
        assert_eq!(
            options.error_info.as_ref().unwrap().as_bytes(),
            b"trace text"
        );
        assert_eq!(
            options.error_code.as_ref().unwrap().as_list_vec().unwrap(),
            vec![VmObj::from_str("A"), VmObj::from_str("B")]
        );
        assert_eq!(options.raw, Some(raw));
        assert_eq!(options.routed_code(), 2);
    }

    #[test]
    fn return_options_reject_odd_and_negative_level() {
        assert!(ReturnOptions::from_vm_obj(VmObj::from_str("-code")).is_err());
        assert!(ReturnOptions::from_vm_obj(VmObj::from_str("-code ok -level -1")).is_err());
    }
}
