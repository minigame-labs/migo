//! Small, allocation-free validators shared by every public ABI record.

use std::ops::RangeInclusive;

use crate::{MIGO_ERROR_INVALID_ARGUMENT, MigoResult};

#[inline]
pub fn validate_flags(value: u64, known_mask: u64) -> Result<(), MigoResult> {
    if value & !known_mask == 0 {
        Ok(())
    } else {
        Err(MIGO_ERROR_INVALID_ARGUMENT)
    }
}

#[inline]
pub fn validate_reserved(value: u64) -> Result<(), MigoResult> {
    if value == 0 {
        Ok(())
    } else {
        Err(MIGO_ERROR_INVALID_ARGUMENT)
    }
}

#[inline]
pub fn validate_bool(value: u8) -> Result<bool, MigoResult> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(MIGO_ERROR_INVALID_ARGUMENT),
    }
}

#[inline]
pub fn validate_f32_range(value: f32, range: RangeInclusive<f32>) -> Result<f32, MigoResult> {
    if value.is_finite()
        && range.start().is_finite()
        && range.end().is_finite()
        && range.start() <= range.end()
        && range.contains(&value)
    {
        Ok(value)
    } else {
        Err(MIGO_ERROR_INVALID_ARGUMENT)
    }
}

#[inline]
pub fn validate_f64_range(value: f64, range: RangeInclusive<f64>) -> Result<f64, MigoResult> {
    if value.is_finite()
        && range.start().is_finite()
        && range.end().is_finite()
        && range.start() <= range.end()
        && range.contains(&value)
    {
        Ok(value)
    } else {
        Err(MIGO_ERROR_INVALID_ARGUMENT)
    }
}

#[inline]
pub fn validate_positive_scale(value: f32) -> Result<f32, MigoResult> {
    if value.is_finite() && value > 0.0 {
        Ok(value)
    } else {
        Err(MIGO_ERROR_INVALID_ARGUMENT)
    }
}
