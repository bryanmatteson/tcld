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
        if let Some(code) = &self.error_code {
            words.push(VmObj::from_str("-errorcode"));
            words.push(code.clone());
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
}

fn parse_completion_code(value: &VmObj) -> Result<i32, VmError> {
    let bytes = value.ensure_bytes()?;
    let code = match bytes.as_bytes() {
        b"ok" => 0,
        b"error" => 1,
        b"return" => 2,
        b"break" => 3,
        b"continue" => 4,
        _ => return Ok(value.as_i64()? as i32),
    };
    Ok(code)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Completion {
    pub code: i32,
    pub result: VmObj,
    pub options: ReturnOptions,
}

impl Completion {
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
