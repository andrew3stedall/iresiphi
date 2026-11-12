//! Exact Decimal128 values. Intermediate coefficients never use binary floats.
use num_bigint::BigInt;
use num_traits::{Signed, ToPrimitive, Zero};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Decimal {
    pub coefficient: i128,
    pub precision: u8,
    pub scale: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rounding {
    HalfEven,
    TowardZero,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScalarError(pub String);
impl fmt::Display for ScalarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ScalarError {}

fn error(message: &str) -> ScalarError {
    ScalarError(message.into())
}
fn power(scale: u8) -> BigInt {
    BigInt::from(10).pow(u32::from(scale))
}
fn schema(precision: u8, scale: u8) -> Result<(), ScalarError> {
    if precision == 0 || precision > 38 || scale > precision {
        return Err(error(
            "decimal requires 1 <= precision <= 38 and scale <= precision",
        ));
    }
    Ok(())
}

impl Decimal {
    fn from_big(coefficient: BigInt, precision: u8, scale: u8) -> Result<Self, ScalarError> {
        schema(precision, scale)?;
        if coefficient.abs() >= power(precision) {
            return Err(error("decimal overflow"));
        }
        Ok(Self {
            coefficient: coefficient
                .to_i128()
                .ok_or_else(|| error("decimal overflow"))?,
            precision,
            scale,
        })
    }

    /// Parse fixed-point decimal text; exponents and surrounding whitespace are rejected.
    pub fn parse(text: &str, precision: u8, scale: u8) -> Result<Self, ScalarError> {
        schema(precision, scale)?;
        let (negative, unsigned) = match text.strip_prefix('-') {
            Some(value) => (true, value),
            None => (false, text.strip_prefix('+').unwrap_or(text)),
        };
        let mut parts = unsigned.split('.');
        let integer = parts.next().unwrap_or("");
        let fraction = parts.next().unwrap_or("");
        if integer.is_empty()
            || !integer.bytes().all(|b| b.is_ascii_digit())
            || !fraction.bytes().all(|b| b.is_ascii_digit())
            || parts.next().is_some()
            || (unsigned.contains('.') && fraction.is_empty())
        {
            return Err(error("malformed decimal"));
        }
        if fraction.len() > usize::from(scale) {
            return Err(error("decimal excess scale requires explicit quantization"));
        }
        // Bound significant input before constructing an arbitrary-precision integer.
        let digits = format!("{integer}{fraction}");
        let significant = digits.trim_start_matches('0');
        if significant.len() > usize::from(precision) {
            return Err(error("decimal overflow"));
        }
        let mut coefficient = if significant.is_empty() {
            BigInt::zero()
        } else {
            BigInt::parse_bytes(significant.as_bytes(), 10)
                .ok_or_else(|| error("malformed decimal"))?
        };
        coefficient *= power(scale - fraction.len() as u8);
        if negative {
            coefficient = -coefficient;
        }
        Self::from_big(coefficient, precision, scale)
    }

    fn valid(self) -> Result<(), ScalarError> {
        Self::from_big(BigInt::from(self.coefficient), self.precision, self.scale).map(|_| ())
    }

    fn exact_rescale(
        coefficient: BigInt,
        source_scale: u8,
        precision: u8,
        scale: u8,
    ) -> Result<Self, ScalarError> {
        schema(precision, scale)?;
        let result = if scale >= source_scale {
            coefficient * power(scale - source_scale)
        } else {
            let divisor = power(source_scale - scale);
            if !(&coefficient % &divisor).is_zero() {
                return Err(error(
                    "inexact decimal rescale requires explicit quantization",
                ));
            }
            coefficient / divisor
        };
        Self::from_big(result, precision, scale)
    }

    fn add_sub(
        self,
        other: Self,
        precision: u8,
        scale: u8,
        subtract: bool,
    ) -> Result<Self, ScalarError> {
        self.valid()?;
        other.valid()?;
        let inferred_scale = self.scale.max(other.scale);
        let inferred_precision =
            (self.precision - self.scale).max(other.precision - other.scale) + inferred_scale + 1;
        schema(inferred_precision, inferred_scale)?;
        let left = BigInt::from(self.coefficient) * power(inferred_scale - self.scale);
        let right = BigInt::from(other.coefficient) * power(inferred_scale - other.scale);
        Self::exact_rescale(
            if subtract { left - right } else { left + right },
            inferred_scale,
            precision,
            scale,
        )
    }
    pub fn checked_add(self, other: Self, precision: u8, scale: u8) -> Result<Self, ScalarError> {
        self.add_sub(other, precision, scale, false)
    }
    pub fn checked_sub(self, other: Self, precision: u8, scale: u8) -> Result<Self, ScalarError> {
        self.add_sub(other, precision, scale, true)
    }
    pub fn checked_mul(self, other: Self, precision: u8, scale: u8) -> Result<Self, ScalarError> {
        self.valid()?;
        other.valid()?;
        schema(self.precision + other.precision, self.scale + other.scale)?;
        Self::exact_rescale(
            BigInt::from(self.coefficient) * BigInt::from(other.coefficient),
            self.scale + other.scale,
            precision,
            scale,
        )
    }
    pub fn quantize(
        self,
        precision: u8,
        scale: u8,
        rounding: Rounding,
    ) -> Result<Self, ScalarError> {
        self.valid()?;
        schema(precision, scale)?;
        if scale >= self.scale {
            return Self::exact_rescale(
                BigInt::from(self.coefficient),
                self.scale,
                precision,
                scale,
            );
        }
        let coefficient = BigInt::from(self.coefficient);
        let divisor = power(self.scale - scale);
        let mut quotient = &coefficient / &divisor;
        let remainder = (&coefficient % &divisor).abs();
        let twice = &remainder * 2;
        if rounding == Rounding::HalfEven
            && (twice > divisor || (twice == divisor && !(&quotient % BigInt::from(2)).is_zero()))
        {
            quotient += if coefficient.is_negative() { -1 } else { 1 };
        }
        Self::from_big(quotient, precision, scale)
    }
}

impl fmt::Display for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let digits = self.coefficient.unsigned_abs().to_string();
        if self.coefficient < 0 {
            f.write_str("-")?;
        }
        if self.scale == 0 {
            return f.write_str(&digits);
        }
        let padded = format!("{:0>width$}", digits, width = usize::from(self.scale) + 1);
        let split = padded.len() - usize::from(self.scale);
        write!(f, "{}.{}", &padded[..split], &padded[split..])
    }
}
