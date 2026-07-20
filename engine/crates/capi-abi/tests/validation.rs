use migo_capi_abi::{
    MIGO_ERROR_INVALID_ARGUMENT,
    validate::{
        validate_bool, validate_f32_range, validate_f64_range, validate_flags,
        validate_positive_scale, validate_reserved,
    },
};

#[test]
fn flags_accept_only_the_known_mask() {
    assert_eq!(validate_flags(0b0011, 0b0011), Ok(()));
    assert_eq!(validate_flags(0, 0b0011), Ok(()));
    assert_eq!(
        validate_flags(0b0100, 0b0011),
        Err(MIGO_ERROR_INVALID_ARGUMENT),
    );
}

#[test]
fn reserved_values_must_be_zero() {
    assert_eq!(validate_reserved(0), Ok(()));
    assert_eq!(validate_reserved(1), Err(MIGO_ERROR_INVALID_ARGUMENT));
    assert_eq!(
        validate_reserved(u64::MAX),
        Err(MIGO_ERROR_INVALID_ARGUMENT),
    );
}

#[test]
fn c_booleans_are_exactly_zero_or_one() {
    assert_eq!(validate_bool(0), Ok(false));
    assert_eq!(validate_bool(1), Ok(true));
    assert_eq!(validate_bool(2), Err(MIGO_ERROR_INVALID_ARGUMENT));
    assert_eq!(validate_bool(u8::MAX), Err(MIGO_ERROR_INVALID_ARGUMENT));
}

#[test]
fn finite_f32_ranges_reject_nan_infinity_and_out_of_range_values() {
    for value in [f32::NAN, f32::INFINITY, -f32::INFINITY, -0.1, 1.1] {
        assert_eq!(
            validate_f32_range(value, 0.0..=1.0),
            Err(MIGO_ERROR_INVALID_ARGUMENT),
        );
    }
    assert_eq!(validate_f32_range(0.0, 0.0..=1.0), Ok(0.0));
    assert_eq!(validate_f32_range(0.5, 0.0..=1.0), Ok(0.5));
    assert_eq!(validate_f32_range(1.0, 0.0..=1.0), Ok(1.0));
}

#[test]
fn finite_f64_ranges_reject_nan_infinity_and_out_of_range_values() {
    for value in [f64::NAN, f64::INFINITY, -f64::INFINITY, -1.01, 1.01] {
        assert_eq!(
            validate_f64_range(value, -1.0..=1.0),
            Err(MIGO_ERROR_INVALID_ARGUMENT),
        );
    }
    assert_eq!(validate_f64_range(-1.0, -1.0..=1.0), Ok(-1.0));
    assert_eq!(validate_f64_range(1.0, -1.0..=1.0), Ok(1.0));
}

#[test]
fn scale_must_be_finite_and_strictly_positive() {
    for value in [0.0, -1.0, f32::NAN, f32::INFINITY, -f32::INFINITY] {
        assert_eq!(
            validate_positive_scale(value),
            Err(MIGO_ERROR_INVALID_ARGUMENT),
        );
    }
    assert_eq!(
        validate_positive_scale(f32::MIN_POSITIVE),
        Ok(f32::MIN_POSITIVE)
    );
    assert_eq!(validate_positive_scale(2.0), Ok(2.0));
}
