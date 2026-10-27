//! Tcl 9's built-in math functions (`tclBasic.c`), computed natively so an
//! expression needs neither the host interpreter nor its `::tcl::mathfunc`
//! commands.

use std::num::FpCategory;
use std::time::{SystemTime, UNIX_EPOCH};

use num_bigint::BigInt;
use num_traits::{FromPrimitive, Signed, ToPrimitive};

use crate::error::VmError;
use crate::number::{self, IntoObj, Number};
use crate::value::VmObj;

/// The largest double that holds every smaller integer exactly.
const MAX_EXACT: f64 = 9_007_199_254_740_991.0;

/// Park–Miller generator behind `rand()` and `srand()`, one per interpreter.
#[derive(Debug, Default)]
pub struct RandState {
    seed: Option<i64>,
}

impl RandState {
    fn next(&mut self) -> f64 {
        let seed = self.seed.get_or_insert_with(|| {
            let clicks = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_micros() as i64);
            normalize_seed(clicks)
        });
        let tmp = *seed / 127_773;
        *seed = 16_807 * (*seed - tmp * 127_773) - 2_836 * tmp;
        if *seed < 0 {
            *seed += 2_147_483_647;
        }
        *seed as f64 * (1.0 / 2_147_483_647.0)
    }
}

/// Keep a seed in 1..=2^31-2, where the recurrence never reaches 0.
fn normalize_seed(value: i64) -> i64 {
    let seed = value & 0x7FFF_FFFF;
    if seed == 0 || seed == 0x7FFF_FFFF {
        seed ^ 123_459_876
    } else {
        seed
    }
}

/// Tcl's names in `::tcl::mathfunc`.
pub const BUILTINS: &[&str] = &[
    "abs",
    "acos",
    "asin",
    "atan",
    "atan2",
    "bool",
    "ceil",
    "cos",
    "cosh",
    "double",
    "entier",
    "exp",
    "floor",
    "fmod",
    "hypot",
    "int",
    "isfinite",
    "isinf",
    "isnan",
    "isnormal",
    "isqrt",
    "issubnormal",
    "isunordered",
    "log",
    "log10",
    "max",
    "min",
    "pow",
    "rand",
    "round",
    "sin",
    "sinh",
    "sqrt",
    "srand",
    "tan",
    "tanh",
    "wide",
];

/// Evaluate `name(args...)`, or `None` when `name` is not a built-in.
pub fn call(name: &str, args: &[VmObj], rand: &mut RandState) -> Option<Result<VmObj, VmError>> {
    BUILTINS.contains(&name).then(|| evaluate(name, args, rand))
}

fn evaluate(name: &str, args: &[VmObj], rand: &mut RandState) -> Result<VmObj, VmError> {
    let arity = |expected: usize| {
        if args.len() == expected {
            Ok(())
        } else {
            Err(wrong_args(name, expected, args.len()))
        }
    };
    let unary: Option<fn(f64) -> f64> = match name {
        "acos" => Some(f64::acos),
        "asin" => Some(f64::asin),
        "atan" => Some(f64::atan),
        "cos" => Some(f64::cos),
        "cosh" => Some(f64::cosh),
        "exp" => Some(f64::exp),
        "log" => Some(f64::ln),
        "log10" => Some(f64::log10),
        "sin" => Some(f64::sin),
        "sinh" => Some(f64::sinh),
        "tan" => Some(f64::tan),
        "tanh" => Some(f64::tanh),
        _ => None,
    };
    if let Some(function) = unary {
        arity(1)?;
        return checked(function(double_arg(&args[0])?));
    }
    let binary: Option<fn(f64, f64) -> f64> = match name {
        "atan2" => Some(f64::atan2),
        "fmod" => Some(|x, y| x % y),
        "hypot" => Some(f64::hypot),
        "pow" => Some(f64::powf),
        _ => None,
    };
    if let Some(function) = binary {
        arity(2)?;
        let x = double_arg(&args[0])?;
        return checked(function(x, double_arg(&args[1])?));
    }
    match name {
        "abs" => {
            arity(1)?;
            abs(&args[0])
        }
        "bool" => {
            arity(1)?;
            Ok(VmObj::Int(i64::from(number::boolean(&args[0])?)))
        }
        "ceil" | "floor" => {
            arity(1)?;
            let d = double_arg(&args[0])?;
            let up = name == "ceil";
            let value = match number::of(&args[0]).and_then(|n| n.to_bigint()) {
                Some(big) => bignum_to_double_toward(&big, up),
                None if up => d.ceil(),
                None => d.floor(),
            };
            Ok(Number::Double(value).into_obj())
        }
        "double" => {
            arity(1)?;
            Ok(Number::Double(double_arg(&args[0])?).into_obj())
        }
        "int" | "entier" => {
            arity(1)?;
            int(&args[0])
        }
        "wide" => {
            arity(1)?;
            let whole = int(&args[0])?;
            let big = number::of(&whole)
                .and_then(|n| n.to_bigint())
                .expect("int() yields an integer");
            let low = (big & BigInt::from(u64::MAX)).to_u64().expect("64 bits") as i64;
            Ok(VmObj::Int(low))
        }
        "round" => {
            arity(1)?;
            round(&args[0])
        }
        "sqrt" => {
            arity(1)?;
            let d = double_arg(&args[0])?;
            if d >= 0.0
                && d.is_infinite()
                && let Some(big) = number::of(&args[0]).and_then(|n| n.to_bigint())
            {
                let root = big.sqrt().to_f64().unwrap_or(f64::INFINITY);
                return Ok(Number::Double(root).into_obj());
            }
            checked(d.sqrt())
        }
        "isqrt" => {
            arity(1)?;
            isqrt(&args[0])
        }
        "max" | "min" => {
            if args.is_empty() {
                return Err(wrong_args(name, 1, 0));
            }
            let wanted = if name == "max" {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Less
            };
            let mut best = &args[0];
            let mut best_number = number_arg(best)?;
            for arg in args {
                let candidate = number_arg(arg)?;
                if candidate.is_nan() {
                    return Err(number::nan_error());
                }
                if number::compare_numbers(&candidate, &best_number) == wanted {
                    best = arg;
                    best_number = candidate;
                }
            }
            Ok(best.clone())
        }
        "isfinite" | "isinf" | "isnan" | "isnormal" | "issubnormal" => {
            arity(1)?;
            let class = fp_class(&args[0])?;
            let holds = match name {
                "isfinite" => class != FpCategory::Infinite && class != FpCategory::Nan,
                "isinf" => class == FpCategory::Infinite,
                "isnan" => class == FpCategory::Nan,
                "isnormal" => class == FpCategory::Normal,
                _ => class == FpCategory::Subnormal,
            };
            Ok(VmObj::Int(i64::from(holds)))
        }
        "isunordered" => {
            arity(2)?;
            let either_nan =
                fp_class(&args[0])? == FpCategory::Nan || fp_class(&args[1])? == FpCategory::Nan;
            Ok(VmObj::Int(i64::from(either_nan)))
        }
        "rand" => {
            arity(0)?;
            Ok(Number::Double(rand.next()).into_obj())
        }
        "srand" => {
            arity(1)?;
            let seed = number::of(&args[0])
                .and_then(|n| n.to_bigint())
                .ok_or_else(|| number::expected("integer", &args[0]))?;
            let low = (seed & BigInt::from(u64::MAX)).to_u64().expect("64 bits") as i64;
            rand.seed = Some(normalize_seed(low));
            Ok(Number::Double(rand.next()).into_obj())
        }
        _ => unreachable!("{name} is listed in BUILTINS"),
    }
}

/// `MathFuncWrongNumArgs`.
fn wrong_args(name: &str, expected: usize, found: usize) -> VmError {
    VmError::TclError {
        message: format!(
            "{} arguments for math function \"{name}\"",
            if found < expected {
                "not enough"
            } else {
                "too many"
            }
        ),
        error_info: None,
        error_code: Some("TCL WRONGARGS".into()),
    }
}

/// `Tcl_GetDoubleFromObj`.
fn double_arg(value: &VmObj) -> Result<f64, VmError> {
    match number::of(value) {
        Some(n) if n.is_nan() => Err(number::nan_error()),
        Some(n) => Ok(n.to_f64()),
        None => Err(number::expected("floating-point number", value)),
    }
}

/// `Tcl_GetNumberFromObj`.
fn number_arg(value: &VmObj) -> Result<Number, VmError> {
    number::of(value).ok_or_else(|| number::expected("number", value))
}

/// `CheckDoubleResult`: NaN is a domain error; infinities stand.
fn checked(value: f64) -> Result<VmObj, VmError> {
    if value.is_nan() {
        Err(number::domain_error())
    } else {
        Ok(Number::Double(value).into_obj())
    }
}

fn integer_too_large() -> VmError {
    let message = "integer value too large to represent";
    VmError::TclError {
        message: message.into(),
        error_info: None,
        error_code: Some(format!("ARITH IOVERFLOW {{{message}}}")),
    }
}

/// `TclCeil`/`TclFloor`: the nearest double at or above (or below) a bignum.
fn bignum_to_double_toward(value: &BigInt, up: bool) -> f64 {
    let d = value.to_f64().unwrap_or(if value.is_negative() {
        f64::NEG_INFINITY
    } else {
        f64::INFINITY
    });
    if !d.is_finite() {
        return d;
    }
    let back = BigInt::from_f64(d).expect("finite double");
    if up && &back < value {
        d.next_up()
    } else if !up && &back > value {
        d.next_down()
    } else {
        d
    }
}

/// `ExprAbsFunc`: integers already non-negative come back unchanged.
fn abs(value: &VmObj) -> Result<VmObj, VmError> {
    match number_arg(value)? {
        Number::Int(i) if i > 0 => Ok(value.clone()),
        Number::Int(0) => {
            let negative_zero = value.ensure_bytes()?.as_bytes().contains(&b'-');
            Ok(if negative_zero {
                VmObj::Int(0)
            } else {
                value.clone()
            })
        }
        Number::Int(i) => Ok(Number::from_bigint(-BigInt::from(i)).into_obj()),
        Number::Double(d) if d.is_nan() => Err(number::nan_error()),
        Number::Double(d) if d > 0.0 || (d == 0.0 && d.is_sign_positive()) => Ok(value.clone()),
        Number::Double(d) => Ok(Number::Double(-d).into_obj()),
        Number::Big(b) if b.is_negative() => Ok(Number::from_bigint(-b).into_obj()),
        Number::Big(_) => Ok(value.clone()),
    }
}

/// `ExprIntFunc` (also `entier`): doubles truncate toward zero.
fn int(value: &VmObj) -> Result<VmObj, VmError> {
    match number_arg(value)? {
        Number::Double(d) if d.is_nan() => Err(number::nan_error()),
        Number::Double(d) if d >= i64::MAX as f64 || d <= i64::MIN as f64 => {
            let whole = BigInt::from_f64(d.trunc()).ok_or_else(integer_too_large)?;
            Ok(Number::from_bigint(whole).into_obj())
        }
        Number::Double(d) => Ok(VmObj::Int(d as i64)),
        _ => Ok(value.clone()),
    }
}

/// `ExprRoundFunc`: halves round away from zero.
fn round(value: &VmObj) -> Result<VmObj, VmError> {
    let d = match number_arg(value)? {
        Number::Double(d) if d.is_nan() => return Err(number::nan_error()),
        Number::Double(d) => d,
        _ => return Ok(value.clone()),
    };
    let whole = d.trunc();
    let fraction = d - whole;
    let step = if fraction <= -0.5 {
        -1
    } else if fraction >= 0.5 {
        1
    } else {
        0
    };
    let (max, min) = (
        i64::MAX - i64::from(step == 1),
        i64::MIN + i64::from(step == -1),
    );
    if whole >= max as f64 || whole <= min as f64 {
        let big = BigInt::from_f64(whole).ok_or_else(integer_too_large)?;
        return Ok(Number::from_bigint(big + step).into_obj());
    }
    Ok(VmObj::Int(whole as i64 + step))
}

/// `ExprIsqrtFunc`: the integer square root, rejecting negatives.
fn isqrt(value: &VmObj) -> Result<VmObj, VmError> {
    let negative = || VmError::TclError {
        message: "square root of negative argument".into(),
        error_info: None,
        error_code: Some("ARITH DOMAIN {domain error: argument not in valid range}".into()),
    };
    let big = match number_arg(value)? {
        Number::Double(d) if d.is_nan() => return Err(number::nan_error()),
        Number::Double(d) if d < 0.0 => return Err(negative()),
        Number::Double(d) if d <= MAX_EXACT => return Ok(VmObj::Int(d.sqrt() as i64)),
        Number::Double(d) => BigInt::from_f64(d.trunc()).ok_or_else(integer_too_large)?,
        Number::Int(i) if i < 0 => return Err(negative()),
        Number::Int(i) if (i as f64) < MAX_EXACT => {
            return Ok(VmObj::Int((i as f64).sqrt() as i64));
        }
        Number::Int(i) => BigInt::from(i),
        Number::Big(b) if b.is_negative() => return Err(negative()),
        Number::Big(b) => b,
    };
    Ok(Number::from_bigint(big.sqrt()).into_obj())
}

/// `DoubleObjClass`.
fn fp_class(value: &VmObj) -> Result<FpCategory, VmError> {
    Ok(match number_arg(value)? {
        Number::Double(d) => d.classify(),
        other => other.to_f64().classify(),
    })
}
