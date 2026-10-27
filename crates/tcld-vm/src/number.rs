//! Tcl's numeric semantics over VM values: arithmetic and comparison as
//! `tclExecute.c` performs them, operand and conversion errors, and boolean
//! conversion (`tclObj.c`). The numeric syntax itself is `tcld_core::number`.

use std::cmp::Ordering;

use num_bigint::BigInt;
use num_traits::{FromPrimitive, One, Signed, ToPrimitive, Zero};

pub use tcld_core::number::{Number, TYPE_BIG, TYPE_DOUBLE, TYPE_INT, TYPE_NAN, format_double};

use crate::error::VmError;
use crate::value::{VmObj, split_tcl_list};

/// A number as the VM value Tcl would hold for it.
pub trait IntoObj {
    fn into_obj(self) -> VmObj;
}

impl IntoObj for Number {
    fn into_obj(self) -> VmObj {
        match self {
            Number::Int(value) => VmObj::Int(value),
            other => VmObj::from_str(&other.to_canonical_string()),
        }
    }
}

/// `GetNumberFromObj`: the numeric value of an operand, if it has one.
pub fn of(value: &VmObj) -> Option<Number> {
    match value {
        VmObj::Int(value) => Some(Number::Int(*value)),
        VmObj::Bytes(bytes) => Number::parse(bytes.as_bytes()),
        VmObj::List(items) if items.len() == 1 => of(&items[0]),
        VmObj::List(_) | VmObj::ProcBody(_) => None,
    }
}

fn error(message: impl Into<String>, error_code: Option<String>) -> VmError {
    VmError::TclError {
        message: message.into(),
        error_info: None,
        error_code,
    }
}

/// `TclParseNumber`'s failure: `expected <what> but got "<text>"`, or
/// `... but got a list` for a multi-element list.
pub fn expected(what: &str, value: &VmObj) -> VmError {
    let text = value.as_string().unwrap_or_default();
    let is_list = match value {
        VmObj::List(items) => items.len() > 1,
        _ => split_tcl_list(text.as_bytes()).is_ok_and(|items| items.len() > 1),
    };
    let got = if is_list {
        "a list".to_string()
    } else {
        format!("\"{}\"", text.chars().take(50).collect::<String>())
    };
    error(
        format!("expected {what} but got {got}"),
        Some("TCL VALUE NUMBER".into()),
    )
}

/// `Tcl_GetDoubleFromObj` on NaN.
pub fn nan_error() -> VmError {
    error(
        "floating point value is Not a Number",
        Some("TCL VALUE DOUBLE NAN".into()),
    )
}

/// `IllegalExprOperandType`. `side` is `"left "`, `"right "` or `""`.
pub fn operand_error(value: &VmObj, side: &str, op: &str) -> VmError {
    let description = match of(value) {
        Some(number) if number.is_nan() => "non-numeric floating-point value",
        Some(Number::Double(_)) => "floating-point value",
        Some(_) => "(big) integer",
        None => {
            let is_list = match value {
                VmObj::List(items) => items.len() > 1,
                VmObj::Bytes(bytes) => {
                    split_tcl_list(bytes.as_bytes()).is_ok_and(|items| items.len() > 1)
                }
                _ => false,
            };
            if is_list {
                return error(
                    format!("cannot use a list as {side}operand of \"{op}\""),
                    Some("ARITH DOMAIN list".into()),
                );
            }
            "non-numeric string"
        }
    };
    let text = value
        .as_string()
        .unwrap_or_else(|_| "<procedure body>".into());
    error(
        format!("cannot use {description} \"{text}\" as {side}operand of \"{op}\""),
        Some(format!("ARITH DOMAIN {{{description}}}")),
    )
}

fn divide_by_zero() -> VmError {
    error(
        "divide by zero",
        Some("ARITH DIVZERO {divide by zero}".into()),
    )
}

/// `TclExprFloatError` for a NaN result.
pub fn domain_error() -> VmError {
    let message = "domain error: argument not in valid range";
    error(message, Some(format!("ARITH DOMAIN {{{message}}}")))
}

fn double_result(value: f64) -> Result<Number, VmError> {
    if value.is_nan() {
        Err(domain_error())
    } else {
        Ok(Number::Double(value))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Shl,
    Shr,
    And,
    Or,
    Xor,
}

impl BinaryOp {
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::Div => "/",
            Self::Mod => "%",
            Self::Pow => "**",
            Self::Shl => "<<",
            Self::Shr => ">>",
            Self::And => "&",
            Self::Or => "|",
            Self::Xor => "^",
        }
    }

    fn integer_only(self) -> bool {
        matches!(
            self,
            Self::Mod | Self::Shl | Self::Shr | Self::And | Self::Or | Self::Xor
        )
    }
}

fn operand(op: BinaryOp, value: &VmObj, side: &str) -> Result<Number, VmError> {
    match of(value) {
        Some(number) if number.is_nan() => Err(operand_error(value, side, op.symbol())),
        Some(Number::Double(_)) if op.integer_only() => {
            Err(operand_error(value, side, op.symbol()))
        }
        Some(number) => Ok(number),
        None => Err(operand_error(value, side, op.symbol())),
    }
}

/// A binary arithmetic or bitwise operator applied as `expr` does.
pub fn binary(op: BinaryOp, left: &VmObj, right: &VmObj) -> Result<VmObj, VmError> {
    let a = operand(op, left, "left ")?;
    let b = operand(op, right, "right ")?;
    Ok(binary_numbers(op, a, b)?.into_obj())
}

pub fn binary_numbers(op: BinaryOp, a: Number, b: Number) -> Result<Number, VmError> {
    match op {
        BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => basic(op, a, b),
        BinaryOp::Mod => modulo(a, b),
        BinaryOp::Pow => power(a, b),
        BinaryOp::Shl | BinaryOp::Shr => shift(op, a, b),
        BinaryOp::And | BinaryOp::Or | BinaryOp::Xor => bitwise(op, a, b),
    }
}

fn basic(op: BinaryOp, a: Number, b: Number) -> Result<Number, VmError> {
    if matches!(a, Number::Double(_)) || matches!(b, Number::Double(_)) {
        let (x, y) = (a.to_f64(), b.to_f64());
        return double_result(match op {
            BinaryOp::Add => x + y,
            BinaryOp::Sub => x - y,
            BinaryOp::Mul => x * y,
            _ => x / y,
        });
    }
    if let (Number::Int(x), Number::Int(y)) = (&a, &b) {
        let (x, y) = (*x, *y);
        let wide = match op {
            BinaryOp::Add => x.checked_add(y),
            BinaryOp::Sub => x.checked_sub(y),
            BinaryOp::Mul => x.checked_mul(y),
            _ => {
                if y == 0 {
                    return Err(divide_by_zero());
                }
                x.checked_div(y).map(|quotient| {
                    if x % y != 0 && ((x < 0) != (y < 0)) {
                        quotient - 1
                    } else {
                        quotient
                    }
                })
            }
        };
        if let Some(value) = wide {
            return Ok(Number::Int(value));
        }
    }
    let (x, y) = (
        a.to_bigint().expect("integer"),
        b.to_bigint().expect("integer"),
    );
    Ok(Number::from_bigint(match op {
        BinaryOp::Add => x + y,
        BinaryOp::Sub => x - y,
        BinaryOp::Mul => x * y,
        _ => {
            if y.is_zero() {
                return Err(divide_by_zero());
            }
            floor_div_rem(&x, &y).0
        }
    }))
}

/// Tcl's integer division: the quotient rounds toward negative infinity and
/// the remainder takes the divisor's sign.
fn floor_div_rem(x: &BigInt, y: &BigInt) -> (BigInt, BigInt) {
    let quotient = x / y;
    let remainder = x % y;
    if !remainder.is_zero() && remainder.is_negative() != y.is_negative() {
        (quotient - 1, remainder + y)
    } else {
        (quotient, remainder)
    }
}

fn modulo(a: Number, b: Number) -> Result<Number, VmError> {
    if let (Number::Int(x), Number::Int(y)) = (&a, &b) {
        let (x, y) = (*x, *y);
        if y == 0 {
            return Err(divide_by_zero());
        }
        if y == 1 || y == -1 || x == 0 {
            return Ok(Number::Int(0));
        }
        let remainder = x % y;
        return Ok(Number::Int(
            if remainder != 0 && ((remainder < 0) != (y < 0)) {
                remainder + y
            } else {
                remainder
            },
        ));
    }
    let (x, y) = (
        a.to_bigint().expect("integer"),
        b.to_bigint().expect("integer"),
    );
    if y.is_zero() {
        return Err(divide_by_zero());
    }
    Ok(Number::from_bigint(floor_div_rem(&x, &y).1))
}

fn power(a: Number, b: Number) -> Result<Number, VmError> {
    let zero_by_negative = || {
        let message = "exponentiation of zero by negative power";
        error(message, Some(format!("ARITH DOMAIN {{{message}}}")))
    };
    if matches!(a, Number::Double(_)) || matches!(b, Number::Double(_)) {
        let (x, y) = (a.to_f64(), b.to_f64());
        if x == 0.0 && y < 0.0 {
            return Err(zero_by_negative());
        }
        return double_result(x.powf(y));
    }
    let exponent = b.to_bigint().expect("integer");
    if exponent.is_zero() {
        return Ok(Number::Int(1));
    }
    if exponent.is_one() {
        return Ok(a);
    }
    let odd = (&exponent % 2u8) != BigInt::zero();
    let base = a.to_bigint().expect("integer");
    if exponent.is_negative() {
        // TIP 123: integers of magnitude above 1 raised to a negative power
        // yield zero.
        return if base.is_zero() {
            Err(zero_by_negative())
        } else if base == BigInt::from(-1) {
            Ok(Number::Int(if odd { -1 } else { 1 }))
        } else if base.is_one() {
            Ok(Number::Int(1))
        } else {
            Ok(Number::Int(0))
        };
    }
    if base.is_zero() || base.is_one() {
        return Ok(Number::from_bigint(base));
    }
    if base == BigInt::from(-1) {
        return Ok(Number::Int(if odd { -1 } else { 1 }));
    }
    match exponent.to_u32().filter(|e| *e < (1 << 28)) {
        Some(exponent) => Ok(Number::from_bigint(base.pow(exponent))),
        None => Err(error("exponent too large", None)),
    }
}

fn shift(op: BinaryOp, a: Number, b: Number) -> Result<Number, VmError> {
    let amount = b.to_bigint().expect("integer");
    if amount.is_negative() {
        return Err(error("negative shift argument", None));
    }
    let value = a.to_bigint().expect("integer");
    if value.is_zero() {
        return Ok(Number::Int(0));
    }
    let amount = amount.to_usize().filter(|n| *n <= i32::MAX as usize);
    if op == BinaryOp::Shl {
        return match amount {
            Some(amount) => Ok(Number::from_bigint(value << amount)),
            None => Err(error("integer value too large to represent", None)),
        };
    }
    Ok(match amount {
        Some(amount) => Number::from_bigint(value >> amount),
        None => Number::Int(if value.is_negative() { -1 } else { 0 }),
    })
}

fn bitwise(op: BinaryOp, a: Number, b: Number) -> Result<Number, VmError> {
    let (x, y) = (
        a.to_bigint().expect("integer"),
        b.to_bigint().expect("integer"),
    );
    Ok(Number::from_bigint(match op {
        BinaryOp::And => x & y,
        BinaryOp::Or => x | y,
        _ => x ^ y,
    }))
}

/// `-x`: NaN and non-numbers are operand errors.
pub fn negate(value: &VmObj) -> Result<VmObj, VmError> {
    match of(value) {
        Some(Number::Double(d)) if !d.is_nan() => Ok(Number::Double(-d).into_obj()),
        Some(Number::Int(i)) if i != i64::MIN => Ok(VmObj::Int(-i)),
        Some(number @ (Number::Int(_) | Number::Big(_))) => {
            Ok(Number::from_bigint(-number.to_bigint().expect("integer")).into_obj())
        }
        _ => Err(operand_error(value, "", "-")),
    }
}

/// `+x`: the canonical form of a number (`+0x10` is `16`).
pub fn plus(value: &VmObj) -> Result<VmObj, VmError> {
    match of(value) {
        Some(number) if !number.is_nan() => Ok(number.into_obj()),
        _ => Err(operand_error(value, "", "+")),
    }
}

/// `~x` on integers.
pub fn bit_not(value: &VmObj) -> Result<VmObj, VmError> {
    match of(value) {
        Some(Number::Int(i)) => Ok(VmObj::Int(!i)),
        Some(Number::Big(b)) => Ok(Number::from_bigint(-b - 1).into_obj()),
        _ => Err(operand_error(value, "", "~")),
    }
}

/// `tryCvtToNumeric`: canonical numbers, NaN as a domain error, and anything
/// else unchanged for a later string interpretation.
pub fn try_numeric(value: VmObj) -> Result<VmObj, VmError> {
    match of(&value) {
        Some(number) if number.is_nan() => Err(domain_error()),
        Some(number) => Ok(number.into_obj()),
        None => Ok(value),
    }
}

/// `numType`.
pub fn type_code(value: &VmObj) -> i64 {
    of(value).map_or(0, |number| number.type_code())
}

/// Ordering of two numbers known not to be NaN (`TclCompareTwoNumbers`),
/// exact across integers, bignums and doubles.
pub fn compare_numbers(a: &Number, b: &Number) -> Ordering {
    match (a, b) {
        (Number::Int(x), Number::Int(y)) => x.cmp(y),
        (Number::Double(x), Number::Double(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        (Number::Double(x), integer) => compare_integer_double(integer, *x).reverse(),
        (integer, Number::Double(y)) => compare_integer_double(integer, *y),
        (x, y) => x
            .to_bigint()
            .expect("integer")
            .cmp(&y.to_bigint().expect("integer")),
    }
}

fn compare_integer_double(integer: &Number, double: f64) -> Ordering {
    if double.is_infinite() {
        return if double > 0.0 {
            Ordering::Less
        } else {
            Ordering::Greater
        };
    }
    let value = integer.to_bigint().expect("integer");
    let floor = BigInt::from_f64(double.floor()).expect("finite double");
    match value.cmp(&floor) {
        Ordering::Equal if double.fract() != 0.0 => Ordering::Less,
        ordering => ordering,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Comparison {
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

impl Comparison {
    fn holds(self, ordering: Ordering) -> bool {
        match self {
            Self::Eq => ordering == Ordering::Equal,
            Self::Ne => ordering != Ordering::Equal,
            Self::Lt => ordering == Ordering::Less,
            Self::Gt => ordering == Ordering::Greater,
            Self::Le => ordering != Ordering::Greater,
            Self::Ge => ordering != Ordering::Less,
        }
    }
}

fn is_empty_string(value: &VmObj) -> bool {
    match value {
        VmObj::Bytes(bytes) => bytes.is_empty(),
        VmObj::List(items) => items.is_empty(),
        _ => false,
    }
}

/// `==`, `<`, …: numeric when both operands are non-empty numbers (NaN is
/// unequal to everything), otherwise a string comparison.
pub fn compare(op: Comparison, left: &VmObj, right: &VmObj) -> Result<bool, VmError> {
    if !is_empty_string(left)
        && !is_empty_string(right)
        && let (Some(a), Some(b)) = (of(left), of(right))
    {
        if a.is_nan() || b.is_nan() {
            return Ok(op == Comparison::Ne);
        }
        return Ok(op.holds(compare_numbers(&a, &b)));
    }
    let (a, b) = (left.ensure_bytes()?, right.ensure_bytes()?);
    Ok(op.holds(a.as_bytes().cmp(b.as_bytes())))
}

/// `ParseBoolean`: `0`, `1`, and unique prefixes of true/false/yes/no/on/off
/// in any case.
fn parse_boolean_word(text: &[u8]) -> Option<bool> {
    match text {
        b"0" => return Some(false),
        b"1" => return Some(true),
        _ => {}
    }
    if text.is_empty() || text.len() > 5 {
        return None;
    }
    let lower = text.to_ascii_lowercase();
    let prefix_of = |word: &[u8]| word.starts_with(&lower);
    match lower[0] {
        b'y' if prefix_of(b"yes") => Some(true),
        b'n' if prefix_of(b"no") => Some(false),
        b't' if prefix_of(b"true") => Some(true),
        b'f' if prefix_of(b"false") => Some(false),
        b'o' if lower.len() >= 2 && prefix_of(b"on") => Some(true),
        b'o' if lower.len() >= 2 && prefix_of(b"off") => Some(false),
        _ => None,
    }
}

/// Whether a value is a boolean literal (`TclSetBooleanFromAny`, used by
/// `tryCvtToBoolean` and `string is boolean`).
pub fn is_boolean_literal(value: &VmObj) -> bool {
    match value {
        VmObj::Int(i) => *i == 0 || *i == 1,
        _ => value
            .ensure_bytes()
            .is_ok_and(|bytes| parse_boolean_word(bytes.as_bytes()).is_some()),
    }
}

/// `Tcl_GetBooleanFromObj`: boolean words or any number (non-zero is true).
pub fn boolean(value: &VmObj) -> Result<bool, VmError> {
    if let VmObj::Int(i) = value {
        return Ok(*i != 0);
    }
    let bytes = value.ensure_bytes()?;
    if let Some(truth) = parse_boolean_word(bytes.as_bytes()) {
        return Ok(truth);
    }
    match Number::parse(bytes.as_bytes()) {
        Some(Number::Double(d)) if d.is_nan() => Err(nan_error()),
        Some(Number::Double(d)) => Ok(d != 0.0),
        Some(Number::Int(i)) => Ok(i != 0),
        Some(Number::Big(_)) => Ok(true),
        None => Err(expected("boolean value", value)),
    }
}

/// `!x`: a non-boolean operand is an operand error.
pub fn logical_not(value: &VmObj) -> Result<VmObj, VmError> {
    boolean(value)
        .map(|truth| VmObj::Int(i64::from(!truth)))
        .map_err(|_| operand_error(value, "", "!"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_division_floors_and_overflows_into_bignums() {
        let int = |op, a: i64, b: i64| {
            binary(op, &VmObj::Int(a), &VmObj::Int(b))
                .unwrap()
                .as_string()
                .unwrap()
        };
        assert_eq!(int(BinaryOp::Div, -7, 2), "-4");
        assert_eq!(int(BinaryOp::Mod, 7, -3), "-2");
        assert_eq!(int(BinaryOp::Add, i64::MAX, 1), "9223372036854775808");
        assert_eq!(int(BinaryOp::Pow, 2, 64), "18446744073709551616");
        assert_eq!(int(BinaryOp::Pow, 2, -1), "0");
    }
}
