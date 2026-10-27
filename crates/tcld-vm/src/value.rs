//! Hybrid value model (design §2.1).

use std::sync::Arc;

use num_bigint::BigInt;
use num_traits::ToPrimitive;

use crate::error::VmError;
use crate::list_codec::TclListCodec;
use crate::load::LoadedCode;

/// Counted-byte string, binary-safe (not Rust `str`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ByteString {
    bytes: Arc<[u8]>,
}

impl ByteString {
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            bytes: Arc::from(bytes.into().into_boxed_slice()),
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn as_str_lossy(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.bytes)
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

impl From<&[u8]> for ByteString {
    fn from(value: &[u8]) -> Self {
        Self::from_bytes(value.to_vec())
    }
}

impl From<&str> for ByteString {
    fn from(value: &str) -> Self {
        Self::from_bytes(value.as_bytes().to_vec())
    }
}

impl From<String> for ByteString {
    fn from(value: String) -> Self {
        Self::from_bytes(value.into_bytes())
    }
}

/// VM value. Not `Tcl_Obj` layout-compatible (marshal at host bridge).
#[derive(Clone)]
pub enum VmObj {
    Bytes(ByteString),
    Int(i64),
    List(Vec<VmObj>),
    /// Nested TEBC body — never marshaled to host (I4).
    ProcBody(Arc<LoadedCode>),
}

impl std::fmt::Debug for VmObj {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bytes(b) => f.debug_tuple("Bytes").field(b).finish(),
            Self::Int(i) => f.debug_tuple("Int").field(i).finish(),
            Self::List(xs) => f.debug_tuple("List").field(xs).finish(),
            Self::ProcBody(code) => f
                .debug_struct("ProcBody")
                .field("code_bytes", &code.bytecode.code_bytes.len())
                .field("num_locals", &code.bytecode.proc_num_locals)
                .finish(),
        }
    }
}

impl PartialEq for VmObj {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Bytes(a), Self::Bytes(b)) => a == b,
            (Self::Int(a), Self::Int(b)) => a == b,
            (Self::List(a), Self::List(b)) => a == b,
            (Self::ProcBody(a), Self::ProcBody(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl VmObj {
    pub fn empty() -> Self {
        Self::Bytes(ByteString::from(""))
    }

    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Self {
        Self::Bytes(ByteString::from_bytes(bytes.into()))
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        Self::Bytes(ByteString::from(s))
    }

    pub fn from_int(i: i64) -> Self {
        Self::Int(i)
    }

    pub fn from_bigint(value: BigInt) -> Self {
        value
            .to_i64()
            .map(Self::Int)
            .unwrap_or_else(|| Self::from_str(&value.to_string()))
    }

    pub fn as_bigint(&self) -> Result<BigInt, VmError> {
        if let Self::ProcBody(_) = self {
            return Err(VmError::BridgeMarshalProcBody);
        }
        crate::number::of(self)
            .and_then(|number| number.to_bigint())
            .ok_or_else(|| crate::number::expected("integer", self))
    }

    pub fn ensure_bytes(&self) -> Result<ByteString, VmError> {
        match self {
            Self::Bytes(b) => Ok(b.clone()),
            Self::Int(i) => Ok(ByteString::from(i.to_string().as_str())),
            Self::List(xs) => Ok(ByteString::from_bytes(list_to_bytes(xs)?)),
            Self::ProcBody(_) => Err(VmError::BridgeMarshalProcBody),
        }
    }

    /// Canonical string form for comparisons / host marshal.
    pub fn as_string(&self) -> Result<String, VmError> {
        Ok(String::from_utf8_lossy(self.ensure_bytes()?.as_bytes()).into_owned())
    }

    pub fn as_i64(&self) -> Result<i64, VmError> {
        match self {
            Self::Int(i) => Ok(*i),
            Self::Bytes(b) => parse_tcl_int(&b.as_str_lossy()).ok_or_else(|| VmError::TclError {
                message: format!("expected integer but got \"{}\"", b.as_str_lossy()),
                error_info: None,
                error_code: Some("ARITH DOMAIN {expected integer}".into()),
            }),
            Self::List(_) => Err(VmError::TclError {
                message: "expected integer but got list".into(),
                error_info: None,
                error_code: None,
            }),
            Self::ProcBody(_) => Err(VmError::BridgeMarshalProcBody),
        }
    }

    /// Tcl boolean truthiness (`Tcl_GetBooleanFromObj`): boolean words or
    /// numbers; anything else is an error.
    pub fn is_true(&self) -> Result<bool, VmError> {
        match self {
            Self::ProcBody(_) => Err(VmError::BridgeMarshalProcBody),
            _ => crate::number::boolean(self),
        }
    }

    pub fn try_as_number(&self) -> Option<f64> {
        match self {
            Self::Int(i) => Some(*i as f64),
            Self::Bytes(b) => parse_tcl_number(&b.as_str_lossy()),
            _ => None,
        }
    }

    pub fn as_list_vec(&self) -> Result<Vec<VmObj>, VmError> {
        match self {
            Self::List(xs) => Ok(xs.clone()),
            Self::Bytes(b) => split_tcl_list(b.as_bytes()),
            Self::Int(i) => Ok(vec![Self::Int(*i)]),
            Self::ProcBody(_) => Err(VmError::BridgeMarshalProcBody),
        }
    }
}

pub fn parse_tcl_int(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(v) = s.parse::<i64>() {
        return Some(v);
    }
    // Allow 0x / 0o / 0b prefixes lightly
    if let Some(rest) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        return i64::from_str_radix(rest, 16).ok();
    }
    None
}

pub fn parse_tcl_bigint(s: &str) -> Option<BigInt> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (negative, unsigned) = trimmed
        .strip_prefix('-')
        .map(|rest| (true, rest))
        .or_else(|| trimmed.strip_prefix('+').map(|rest| (false, rest)))
        .unwrap_or((false, trimmed));
    let (radix, digits) = if let Some(rest) = unsigned
        .strip_prefix("0x")
        .or_else(|| unsigned.strip_prefix("0X"))
    {
        (16, rest)
    } else if let Some(rest) = unsigned
        .strip_prefix("0o")
        .or_else(|| unsigned.strip_prefix("0O"))
    {
        (8, rest)
    } else if let Some(rest) = unsigned
        .strip_prefix("0b")
        .or_else(|| unsigned.strip_prefix("0B"))
    {
        (2, rest)
    } else {
        (10, unsigned)
    };
    let value = BigInt::parse_bytes(digits.as_bytes(), radix)?;
    Some(if negative { -value } else { value })
}

pub fn parse_tcl_number(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(i) = parse_tcl_int(s) {
        return Some(i as f64);
    }
    s.parse::<f64>().ok()
}

/// Space-joined Tcl list string (MVP string-rep marshal).
pub fn list_to_string(xs: &[VmObj]) -> Result<String, VmError> {
    Ok(String::from_utf8_lossy(&list_to_bytes(xs)?).into_owned())
}

pub fn list_to_bytes(xs: &[VmObj]) -> Result<Vec<u8>, VmError> {
    let mut elements = Vec::with_capacity(xs.len());
    for value in xs {
        elements.push(value.ensure_bytes()?.as_bytes().to_vec());
    }
    Ok(TclListCodec::format(&elements))
}

pub fn split_tcl_list(bytes: &[u8]) -> Result<Vec<VmObj>, VmError> {
    Ok(TclListCodec::parse(bytes)?
        .into_iter()
        .map(VmObj::from_bytes)
        .collect())
}

/// Compare for TEBC `eq`/`neq`/`lt`/… — prefer numeric when both numeric.
pub fn cmp_values(a: &VmObj, b: &VmObj) -> Result<std::cmp::Ordering, VmError> {
    if let (Some(na), Some(nb)) = (a.try_as_number(), b.try_as_number()) {
        return Ok(na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal));
    }
    let sa = a.as_string()?;
    let sb = b.as_string()?;
    Ok(sa.cmp(&sb))
}

/// `Tcl_StringCaseMatch`: glob matching with `*`, `?`, `[...]` sets and
/// ranges, and `\` escapes; `nocase` compares Tcl's lowercase mappings.
pub fn glob_match(pattern: &str, string: &str, nocase: bool) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let string: Vec<char> = string.chars().collect();
    glob_match_chars(&pattern, &string, nocase)
}

/// `TclMatchIsTrivial`: a pattern without glob characters matches only
/// itself.
pub fn match_is_trivial(pattern: &str) -> bool {
    !pattern.contains(['*', '?', '[', '\\'])
}

fn glob_match_chars(pattern: &[char], string: &[char], nocase: bool) -> bool {
    let fold = |c: char| {
        if nocase {
            crate::unicode::to_lower(c)
        } else {
            c
        }
    };
    let (mut p, mut s) = (0, 0);
    loop {
        let Some(&next) = pattern.get(p) else {
            return s == string.len();
        };
        if s == string.len() && next != '*' {
            return false;
        }
        match next {
            '*' => {
                while pattern.get(p) == Some(&'*') {
                    p += 1;
                }
                if p == pattern.len() {
                    return true;
                }
                loop {
                    if glob_match_chars(&pattern[p..], &string[s..], nocase) {
                        return true;
                    }
                    if s == string.len() {
                        return false;
                    }
                    s += 1;
                }
            }
            '?' => {
                p += 1;
                s += 1;
            }
            '[' => {
                p += 1;
                let ch = fold(string[s]);
                s += 1;
                loop {
                    let start = match pattern.get(p) {
                        None | Some(']') => return false,
                        Some(&c) => fold(c),
                    };
                    p += 1;
                    if pattern.get(p) == Some(&'-') {
                        p += 1;
                        let Some(&end) = pattern.get(p) else {
                            return false;
                        };
                        let end = fold(end);
                        p += 1;
                        if (start <= ch && ch <= end) || (end <= ch && ch <= start) {
                            break;
                        }
                    } else if start == ch {
                        break;
                    }
                }
                // Matched: skip the rest of the set. An unclosed set
                // matches only at the end of the string.
                loop {
                    match pattern.get(p) {
                        None => return s == string.len(),
                        Some(']') => break,
                        Some(_) => p += 1,
                    }
                }
                p += 1;
            }
            _ => {
                if next == '\\' {
                    p += 1;
                    if p == pattern.len() {
                        return false;
                    }
                }
                if fold(string[s]) != fold(pattern[p]) {
                    return false;
                }
                p += 1;
                s += 1;
            }
        }
    }
}

/// Parse `name(element)` array form → (name1, Some(name2)).
pub fn parse_array_name(name: &[u8]) -> (ByteString, Option<ByteString>) {
    if let Some(open) = name.iter().position(|&b| b == b'(')
        && name.last() == Some(&b')')
        && open + 1 < name.len()
    {
        let n1 = ByteString::from_bytes(name[..open].to_vec());
        let n2 = ByteString::from_bytes(name[open + 1..name.len() - 1].to_vec());
        return (n1, Some(n2));
    }
    (ByteString::from_bytes(name.to_vec()), None)
}
