//! Tcl 9's numeric syntax: 64-bit integers that overflow into bignums, IEEE
//! doubles, parsing as `TclParseNumber` does (`tclStrToD.c`), and the
//! canonical strings Tcl generates (`Tcl_PrintDouble` in `tclUtil.c`).

use num_bigint::BigInt;
use num_traits::{Signed, ToPrimitive};

/// `TCL_NUMBER_*` codes, as `numType` reports them (0 for non-numeric).
pub const TYPE_INT: i64 = 2;
pub const TYPE_BIG: i64 = 3;
pub const TYPE_DOUBLE: i64 = 4;
pub const TYPE_NAN: i64 = 5;

#[derive(Debug, Clone)]
pub enum Number {
    Int(i64),
    /// Never representable as `i64`.
    Big(BigInt),
    Double(f64),
}

impl Number {
    pub fn from_bigint(value: BigInt) -> Self {
        value.to_i64().map_or(Self::Big(value), Self::Int)
    }

    /// Parse a whole string the way `TclParseNumber` does for an expression
    /// operand: surrounding whitespace, an optional sign, `0x`/`0o`/`0b`/`0d`
    /// prefixes, `_` between digits, decimal integers (leading zeros stay
    /// decimal in Tcl 9), doubles, and `Inf`/`Infinity`/`NaN`.
    pub fn parse(text: &[u8]) -> Option<Self> {
        let start = text.iter().position(|b| !is_space(*b))?;
        let end = text.iter().rposition(|b| !is_space(*b))? + 1;
        let text = &text[start..end];
        let (negative, body) = match text.first()? {
            b'-' => (true, &text[1..]),
            b'+' => (false, &text[1..]),
            _ => (false, text),
        };
        if body.is_empty() {
            return None;
        }
        if let Some(special) = parse_special(body) {
            return Some(Self::Double(if negative { -special } else { special }));
        }
        let signed = |value: BigInt| Self::from_bigint(if negative { -value } else { value });
        if body.len() >= 2 && body[0] == b'0' {
            let radix = match body[1] {
                b'x' | b'X' => Some(16),
                b'o' | b'O' => Some(8),
                b'b' | b'B' => Some(2),
                b'd' | b'D' => Some(10),
                _ => None,
            };
            if let Some(radix) = radix {
                let digits = strip_digit_separators(&body[2..], |b| (b as char).is_digit(radix))?;
                if digits.is_empty() {
                    return None;
                }
                return BigInt::parse_bytes(&digits, radix).map(signed);
            }
        }
        let decimal = strip_digit_separators(body, |b| b.is_ascii_digit())?;
        if decimal.iter().all(u8::is_ascii_digit) {
            return BigInt::parse_bytes(&decimal, 10).map(signed);
        }
        if !is_decimal_double(&decimal) {
            return None;
        }
        let value: f64 = std::str::from_utf8(&decimal).ok()?.parse().ok()?;
        Some(Self::Double(if negative { -value } else { value }))
    }

    pub fn type_code(&self) -> i64 {
        match self {
            Self::Int(_) => TYPE_INT,
            Self::Big(_) => TYPE_BIG,
            Self::Double(value) if value.is_nan() => TYPE_NAN,
            Self::Double(_) => TYPE_DOUBLE,
        }
    }

    pub fn is_nan(&self) -> bool {
        matches!(self, Self::Double(value) if value.is_nan())
    }

    pub fn to_f64(&self) -> f64 {
        match self {
            Self::Int(value) => *value as f64,
            Self::Big(value) => value.to_f64().unwrap_or(if value.is_negative() {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            }),
            Self::Double(value) => *value,
        }
    }

    /// The integer value, or `None` for a double.
    pub fn to_bigint(&self) -> Option<BigInt> {
        match self {
            Self::Int(value) => Some(BigInt::from(*value)),
            Self::Big(value) => Some(value.clone()),
            Self::Double(_) => None,
        }
    }

    /// The string Tcl generates for the value.
    pub fn to_canonical_string(&self) -> String {
        match self {
            Self::Int(value) => value.to_string(),
            Self::Big(value) => value.to_string(),
            Self::Double(value) => format_double(*value),
        }
    }
}

fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t'..=b'\r')
}

fn parse_special(body: &[u8]) -> Option<f64> {
    let lower = body.to_ascii_lowercase();
    if lower == b"inf" || lower == b"infinity" {
        return Some(f64::INFINITY);
    }
    let rest = lower.strip_prefix(b"nan")?;
    let payload_ok = rest.is_empty()
        || (rest.len() >= 3
            && rest[0] == b'('
            && rest[rest.len() - 1] == b')'
            && rest[1..rest.len() - 1].iter().all(u8::is_ascii_hexdigit));
    payload_ok.then_some(f64::NAN)
}

/// Remove `_` digit separators, which Tcl allows only between two digits.
fn strip_digit_separators(text: &[u8], is_digit: impl Fn(u8) -> bool) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len());
    for (index, &byte) in text.iter().enumerate() {
        if byte != b'_' {
            out.push(byte);
            continue;
        }
        let before = text[..index].iter().rev().find(|b| **b != b'_');
        let after = text[index + 1..].iter().find(|b| **b != b'_');
        if index == 0
            || !before.is_some_and(|b| is_digit(*b))
            || !after.is_some_and(|b| is_digit(*b))
        {
            return None;
        }
    }
    Some(out)
}

/// `digits [. digits] [e [+-] digits]` with at least one significand digit.
fn is_decimal_double(text: &[u8]) -> bool {
    let mut i = 0;
    let digits = |i: &mut usize| {
        let start = *i;
        while text.get(*i).is_some_and(u8::is_ascii_digit) {
            *i += 1;
        }
        *i - start
    };
    let mut significand = digits(&mut i);
    if text.get(i) == Some(&b'.') {
        i += 1;
        significand += digits(&mut i);
    }
    if significand == 0 {
        return false;
    }
    if matches!(text.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(text.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        if digits(&mut i) == 0 {
            return false;
        }
    }
    i == text.len()
}

/// `Tcl_PrintDouble`: the shortest digits that round-trip, in fixed notation
/// for decimal exponents -4..=16 and `d.ddde±x` otherwise, with integral
/// values keeping a `.0`.
pub fn format_double(value: f64) -> String {
    if value.is_nan() {
        return "NaN".into();
    }
    if value.is_infinite() {
        return if value < 0.0 { "-Inf" } else { "Inf" }.into();
    }
    let scientific = format!("{value:e}");
    let (mantissa, exponent) = scientific.split_once('e').expect("scientific notation");
    let exponent: i32 = exponent.parse().expect("decimal exponent");
    let digits: Vec<char> = mantissa.chars().filter(char::is_ascii_digit).collect();
    let mut out = String::with_capacity(digits.len() + 8);
    if mantissa.starts_with('-') {
        out.push('-');
    }
    if !(-4..=16).contains(&exponent) {
        out.push(digits[0]);
        if digits.len() > 1 {
            out.push('.');
            out.extend(&digits[1..]);
        }
        out.push_str(&format!("e{exponent:+}"));
    } else if exponent < 0 {
        out.push_str("0.");
        out.extend(std::iter::repeat_n('0', (-exponent - 1) as usize));
        out.extend(&digits);
    } else {
        let whole = exponent as usize + 1;
        for index in 0..whole {
            out.push(digits.get(index).copied().unwrap_or('0'));
        }
        out.push('.');
        if digits.len() > whole {
            out.extend(&digits[whole..]);
        } else {
            out.push('0');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> Option<String> {
        Number::parse(text.as_bytes()).map(|n| n.to_canonical_string())
    }

    #[test]
    fn parses_tcl9_numeric_syntax() {
        assert_eq!(parsed(" 12 ").as_deref(), Some("12"));
        assert_eq!(parsed("010").as_deref(), Some("10"));
        assert_eq!(parsed("0x1F").as_deref(), Some("31"));
        assert_eq!(parsed("-0b101").as_deref(), Some("-5"));
        assert_eq!(parsed("0o17").as_deref(), Some("15"));
        assert_eq!(parsed("1_000_000").as_deref(), Some("1000000"));
        assert_eq!(parsed("1.5e3").as_deref(), Some("1500.0"));
        assert_eq!(parsed(".5").as_deref(), Some("0.5"));
        assert_eq!(parsed("5.").as_deref(), Some("5.0"));
        assert_eq!(parsed("-Inf").as_deref(), Some("-Inf"));
        assert_eq!(parsed("nan").as_deref(), Some("NaN"));
        assert_eq!(
            parsed("99999999999999999999").as_deref(),
            Some("99999999999999999999")
        );
        for bad in ["", " ", "-", "1_", "_1", "0x", "1e", "1.2.3", "abc", "0x_1"] {
            assert!(parsed(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn formats_doubles_like_tcl_print_double() {
        for (value, text) in [
            (3.0, "3.0"),
            (0.1 + 0.2, "0.30000000000000004"),
            (1e21, "1e+21"),
            (1e17, "1e+17"),
            (1e16, "10000000000000000.0"),
            (1.5e-5, "1.5e-5"),
            (0.0001, "0.0001"),
            (-0.0, "-0.0"),
            (123.456, "123.456"),
        ] {
            assert_eq!(format_double(value), text);
        }
    }
}
