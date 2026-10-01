//! .NET `System.Decimal` compatibility surface.
//!
//! `rust_decimal` deliberately uses the same 96-bit integer + sign + scale
//! representation as .NET.  This wrapper keeps bit construction and checked
//! arithmetic explicit; callers must handle overflow instead of falling back
//! to binary floating point.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct DotNetDecimalBits {
    pub lo: u32,
    pub mid: u32,
    pub hi: u32,
    pub negative: bool,
    pub scale: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct DotNetDecimal(Decimal);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecimalError {
    InvalidScale(u32),
    Overflow,
    DivisionByZero,
}

impl fmt::Display for DecimalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidScale(scale) => write!(f, ".NET Decimal scale {scale} exceeds 28"),
            Self::Overflow => write!(f, ".NET Decimal arithmetic overflow"),
            Self::DivisionByZero => write!(f, ".NET Decimal division by zero"),
        }
    }
}

impl std::error::Error for DecimalError {}

impl DotNetDecimal {
    const MAX_MANTISSA: u128 = (1_u128 << 96) - 1;

    pub fn from_bits(bits: DotNetDecimalBits) -> Result<Self, DecimalError> {
        if bits.scale > 28 {
            return Err(DecimalError::InvalidScale(bits.scale));
        }
        Ok(Self(Decimal::from_parts(
            bits.lo,
            bits.mid,
            bits.hi,
            bits.negative,
            bits.scale,
        )))
    }

    pub fn from_i64(value: i64) -> Self {
        Self(Decimal::from(value))
    }

    /// Exact zero.
    pub fn zero() -> Self {
        Self(Decimal::ZERO)
    }

    /// The exact ratio `numerator / denominator`.
    ///
    /// The damage pipeline's multipliers are literal ratios in the IL
    /// (Vulnerable's 3/2, Weak's 3/4, Shrink's 7/10); building them here keeps
    /// the call sites reading like the IL rather than like a magic constant.
    pub fn ratio(numerator: i64, denominator: i64) -> Result<Self, DecimalError> {
        Self::from_i64(numerator).checked_div(Self::from_i64(denominator))
    }

    /// Build one nonnegative rational only when .NET Decimal can retain it
    /// exactly. Canonical Python `Fraction`s with other prime factors would
    /// otherwise be silently rounded by Decimal division.
    pub(crate) fn from_nonnegative_fraction(
        mut numerator: u128,
        mut denominator: u128,
    ) -> Result<Self, DecimalError> {
        if denominator == 0 {
            return Err(DecimalError::DivisionByZero);
        }
        let divisor = gcd(numerator, denominator);
        numerator /= divisor;
        denominator /= divisor;

        let mut twos = 0_u32;
        while denominator.is_multiple_of(2) {
            denominator /= 2;
            twos += 1;
        }
        let mut fives = 0_u32;
        while denominator.is_multiple_of(5) {
            denominator /= 5;
            fives += 1;
        }
        if denominator != 1 {
            return Err(DecimalError::Overflow);
        }
        let scale = twos.max(fives);
        if scale > 28 {
            return Err(DecimalError::InvalidScale(scale));
        }
        for _ in twos..scale {
            numerator = numerator.checked_mul(2).ok_or(DecimalError::Overflow)?;
        }
        for _ in fives..scale {
            numerator = numerator.checked_mul(5).ok_or(DecimalError::Overflow)?;
        }
        if numerator > Self::MAX_MANTISSA {
            return Err(DecimalError::Overflow);
        }
        Self::from_bits(DotNetDecimalBits {
            lo: numerator as u32,
            mid: (numerator >> 32) as u32,
            hi: (numerator >> 64) as u32,
            negative: false,
            scale,
        })
    }

    /// The exact reduced rational represented by these Decimal bits.
    pub(crate) fn nonnegative_fraction(self) -> Option<(u128, u128)> {
        let bits = self.bits();
        if bits.negative {
            return None;
        }
        let numerator =
            u128::from(bits.lo) | (u128::from(bits.mid) << 32) | (u128::from(bits.hi) << 64);
        let denominator = 10_u128.checked_pow(bits.scale)?;
        let divisor = gcd(numerator, denominator);
        Some((numerator / divisor, denominator / divisor))
    }

    pub fn bits(self) -> DotNetDecimalBits {
        let unpacked = self.0.unpack();
        DotNetDecimalBits {
            lo: unpacked.lo,
            mid: unpacked.mid,
            hi: unpacked.hi,
            negative: unpacked.negative,
            scale: unpacked.scale,
        }
    }

    pub fn checked_add(self, rhs: Self) -> Result<Self, DecimalError> {
        self.0
            .checked_add(rhs.0)
            .map(Self)
            .ok_or(DecimalError::Overflow)
    }

    pub fn checked_sub(self, rhs: Self) -> Result<Self, DecimalError> {
        self.0
            .checked_sub(rhs.0)
            .map(Self)
            .ok_or(DecimalError::Overflow)
    }

    pub fn checked_mul(self, rhs: Self) -> Result<Self, DecimalError> {
        self.0
            .checked_mul(rhs.0)
            .map(Self)
            .ok_or(DecimalError::Overflow)
    }

    pub fn checked_div(self, rhs: Self) -> Result<Self, DecimalError> {
        if rhs.0.is_zero() {
            return Err(DecimalError::DivisionByZero);
        }
        self.0
            .checked_div(rhs.0)
            .map(Self)
            .ok_or(DecimalError::Overflow)
    }

    /// .NET explicit Decimal -> integer conversion truncates toward zero.
    pub fn trunc_i64(self) -> Result<i64, DecimalError> {
        use rust_decimal::prelude::ToPrimitive;
        self.0.trunc().to_i64().ok_or(DecimalError::Overflow)
    }

    pub fn canonical_string(self) -> String {
        self.0.to_string()
    }
}

fn gcd(mut left: u128, mut right: u128) -> u128 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decimal(coefficient: u32, scale: u32, negative: bool) -> DotNetDecimal {
        DotNetDecimal::from_bits(DotNetDecimalBits {
            lo: coefficient,
            mid: 0,
            hi: 0,
            negative,
            scale,
        })
        .unwrap()
    }

    #[test]
    fn preserves_dotnet_bits_including_scale_and_negative_zero() {
        let value = DotNetDecimal::from_bits(DotNetDecimalBits {
            lo: 15,
            mid: 0,
            hi: 0,
            negative: true,
            scale: 1,
        })
        .unwrap();
        assert_eq!(
            value.bits(),
            DotNetDecimalBits {
                lo: 15,
                mid: 0,
                hi: 0,
                negative: true,
                scale: 1,
            }
        );
    }

    #[test]
    fn arithmetic_and_explicit_integer_conversion_match_dotnet_vectors() {
        assert_eq!(decimal(405, 1, false).trunc_i64().unwrap(), 40);
        assert_eq!(decimal(405, 1, true).trunc_i64().unwrap(), -40);
        assert_eq!(
            decimal(15, 1, false)
                .checked_add(decimal(225, 2, false))
                .unwrap()
                .canonical_string(),
            "3.75"
        );
        assert_eq!(
            decimal(125, 2, false)
                .checked_mul(decimal(8, 1, false))
                .unwrap()
                .canonical_string(),
            "1.000"
        );
    }

    #[test]
    fn rejects_invalid_or_unrepresentable_operations() {
        assert_eq!(
            DotNetDecimal::from_bits(DotNetDecimalBits {
                lo: 1,
                mid: 0,
                hi: 0,
                negative: false,
                scale: 29,
            }),
            Err(DecimalError::InvalidScale(29))
        );
        assert_eq!(
            DotNetDecimal::from_i64(1).checked_div(DotNetDecimal::from_i64(0)),
            Err(DecimalError::DivisionByZero)
        );
    }

    #[test]
    fn exact_fraction_conversion_never_rounds() {
        let value = DotNetDecimal::from_nonnegative_fraction(231, 40).unwrap();
        assert_eq!(value.nonnegative_fraction(), Some((231, 40)));
        assert_eq!(value.canonical_string(), "5.775");
        assert_eq!(
            DotNetDecimal::from_nonnegative_fraction(7, 3),
            Err(DecimalError::Overflow)
        );
    }
}
