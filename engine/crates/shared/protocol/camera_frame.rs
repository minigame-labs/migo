//! Camera preview frame packing.
//!
//! A `YUV_420_888` camera frame arrives as three separate `Image.Plane`
//! `ByteBuffer`s (Y, U, V). The JS-visible contract is a single flat
//! `ArrayBuffer` = each plane's `position..limit` window concatenated in
//! **Y, U, V** order. This module performs the one unavoidable copy that
//! flattens the three non-contiguous plane windows into one owned `Vec<u8>`.
//!
//! It is deliberately safe and free of JNI/V8 dependencies so it is unit
//! testable on the host: the JNI layer resolves each direct buffer's full
//! capacity slice and passes it here together with the `(offset, length)`
//! window (still as signed `jint` values), and this validates the window with
//! checked arithmetic before copying only the validated sub-slice. Plane
//! padding / pixel-stride bytes inside a window are copied verbatim — this does
//! not repack, convert, or reinterpret the layout.

/// One plane's contribution: a validated `[offset, offset + len)` window of
/// `buffer`, where `buffer` is the plane's full direct-buffer capacity slice
/// and `offset`/`len` are the buffer's `position`/`remaining` (signed, exactly
/// as they cross JNI).
pub struct PlaneWindow<'a> {
    /// The plane's full direct-buffer capacity (index 0..capacity).
    pub buffer: &'a [u8],
    /// Window start (`ByteBuffer.position()`); must be `>= 0`.
    pub offset: i32,
    /// Window length (`ByteBuffer.remaining()`); must be `>= 0`.
    pub len: i32,
}

/// Why a camera frame could not be packed. Returned instead of panicking so a
/// malformed frame from the platform is dropped, never crashing the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraFramePackError {
    /// A plane's offset was negative.
    NegativeOffset,
    /// A plane's length was negative.
    NegativeLength,
    /// `offset + len` overflowed or exceeded the buffer capacity.
    WindowOutOfBounds,
    /// The summed length of the three windows overflowed `usize`.
    TotalOverflow,
    /// Allocation of the exact output buffer failed.
    AllocFailed,
}

/// Pack the three plane windows into one exactly-reserved `Vec<u8>` in Y/U/V
/// order. Validates each window (non-negative offset/length, `offset + len`
/// within capacity, no overflow) with checked arithmetic. Source slices are
/// never mutated.
pub fn pack_yuv_planes(planes: [PlaneWindow<'_>; 3]) -> Result<Vec<u8>, CameraFramePackError> {
    let y = validate_window(&planes[0])?;
    let u = validate_window(&planes[1])?;
    let v = validate_window(&planes[2])?;

    let total = checked_total(y.len(), u.len(), v.len())?;

    let mut out = Vec::new();
    out.try_reserve_exact(total)
        .map_err(|_| CameraFramePackError::AllocFailed)?;
    out.extend_from_slice(y);
    out.extend_from_slice(u);
    out.extend_from_slice(v);
    Ok(out)
}

/// Checked sum of the three plane window lengths, mapping overflow directly to
/// [`CameraFramePackError::TotalOverflow`]. Extracted so the exact production
/// error mapping (`pack_yuv_planes` uses `checked_total(...)?`) is unit-testable
/// on raw `usize` values without allocating `usize`-sized slices (impossible in
/// practice — real camera planes are far smaller).
fn checked_total(y_len: usize, u_len: usize, v_len: usize) -> Result<usize, CameraFramePackError> {
    y_len
        .checked_add(u_len)
        .and_then(|s| s.checked_add(v_len))
        .ok_or(CameraFramePackError::TotalOverflow)
}

/// Validate one window against its buffer capacity and return the validated
/// `[offset, offset + len)` sub-slice. Rejects negative offset/length and any
/// window that overflows or reaches past the buffer.
fn validate_window<'a>(plane: &PlaneWindow<'a>) -> Result<&'a [u8], CameraFramePackError> {
    if plane.offset < 0 {
        return Err(CameraFramePackError::NegativeOffset);
    }
    if plane.len < 0 {
        return Err(CameraFramePackError::NegativeLength);
    }
    let offset = plane.offset as usize;
    let len = plane.len as usize;
    // `checked_add` is defense-in-depth: with `i32` inputs on a 64-bit target
    // this cannot overflow `usize`, but it costs nothing and stays correct if
    // the input widths ever change.
    let end = offset
        .checked_add(len)
        .ok_or(CameraFramePackError::WindowOutOfBounds)?;
    if end > plane.buffer.len() {
        return Err(CameraFramePackError::WindowOutOfBounds);
    }
    Ok(&plane.buffer[offset..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(buffer: &[u8], offset: i32, len: i32) -> PlaneWindow<'_> {
        PlaneWindow {
            buffer,
            offset,
            len,
        }
    }

    #[test]
    fn packs_three_windows_in_yuv_order_from_zero_offsets() {
        let y = [1u8, 2, 3];
        let u = [4u8, 5];
        let v = [6u8];
        let out = pack_yuv_planes([win(&y, 0, 3), win(&u, 0, 2), win(&v, 0, 1)]).unwrap();
        assert_eq!(out, [1, 2, 3, 4, 5, 6], "Y then U then V, in order");
    }

    #[test]
    fn packs_non_zero_offset_windows() {
        // Each buffer has bytes outside the window that must be excluded.
        let y = [9u8, 1, 2, 3, 9]; // window [1..4) = [1,2,3]
        let u = [9u8, 9, 4, 5]; // window [2..4) = [4,5]
        let v = [6u8, 9]; // window [0..1) = [6]
        let out = pack_yuv_planes([win(&y, 1, 3), win(&u, 2, 2), win(&v, 0, 1)]).unwrap();
        assert_eq!(out, [1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn exact_end_window_is_accepted() {
        // offset + len == capacity is valid (the last byte is included).
        let y = [1u8, 2, 3, 4];
        let out = pack_yuv_planes([win(&y, 2, 2), win(&[], 0, 0), win(&[], 0, 0)]).unwrap();
        assert_eq!(out, [3, 4]);
    }

    #[test]
    fn zero_length_windows_yield_empty_contribution() {
        // A zero-length window at any (in-bounds) offset contributes nothing.
        let y = [1u8, 2, 3];
        let out = pack_yuv_planes([win(&y, 0, 0), win(&y, 3, 0), win(&y, 1, 0)]).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn rejects_negative_offset() {
        let y = [1u8, 2, 3];
        assert_eq!(
            pack_yuv_planes([win(&y, -1, 1), win(&[], 0, 0), win(&[], 0, 0)]),
            Err(CameraFramePackError::NegativeOffset)
        );
    }

    #[test]
    fn rejects_negative_length() {
        let y = [1u8, 2, 3];
        assert_eq!(
            pack_yuv_planes([win(&y, 0, -1), win(&[], 0, 0), win(&[], 0, 0)]),
            Err(CameraFramePackError::NegativeLength)
        );
    }

    #[test]
    fn rejects_window_beyond_capacity() {
        let y = [1u8, 2, 3];
        // offset + len = 5 > capacity 3
        assert_eq!(
            pack_yuv_planes([win(&y, 0, 5), win(&[], 0, 0), win(&[], 0, 0)]),
            Err(CameraFramePackError::WindowOutOfBounds)
        );
        // offset alone beyond capacity
        assert_eq!(
            pack_yuv_planes([win(&y, 4, 0), win(&[], 0, 0), win(&[], 0, 0)]),
            Err(CameraFramePackError::WindowOutOfBounds)
        );
    }

    #[test]
    fn rejects_i32_max_window_as_out_of_bounds() {
        // With `i32` inputs on a 64-bit target, `offset + len` (max ~2^32)
        // cannot overflow `usize`, so this hits the out-of-capacity check, not
        // the `checked_add` guard. The guard is belt-and-suspenders; the real
        // usize-overflow path is covered by `checked_total_detects_usize_overflow`.
        let y = [1u8, 2, 3];
        assert_eq!(
            pack_yuv_planes([win(&y, i32::MAX, i32::MAX), win(&[], 0, 0), win(&[], 0, 0)]),
            Err(CameraFramePackError::WindowOutOfBounds)
        );
    }

    #[test]
    fn checked_total_maps_usize_overflow_to_total_overflow_error() {
        // Exercises the exact production error mapping: `pack_yuv_planes` calls
        // `checked_total(...)?`, so this asserts the same `TotalOverflow` value
        // the pack path would return. Unreachable with real camera planes.
        assert_eq!(checked_total(1, 2, 3), Ok(6));
        assert_eq!(checked_total(usize::MAX, 0, 0), Ok(usize::MAX));
        assert_eq!(
            checked_total(usize::MAX, 1, 0),
            Err(CameraFramePackError::TotalOverflow),
            "first add overflows"
        );
        assert_eq!(
            checked_total(usize::MAX - 1, 1, 1),
            Err(CameraFramePackError::TotalOverflow),
            "second add overflows"
        );
    }

    #[test]
    fn padding_and_pixel_stride_bytes_are_preserved_verbatim() {
        // Bytes that look like row padding / pixelStride gaps (0xFF, 0x00) are
        // inside the window and must be copied as-is, not stripped or repacked.
        let y = [10u8, 0xFF, 0x00, 11, 0xFF, 12];
        let out = pack_yuv_planes([win(&y, 0, 6), win(&[], 0, 0), win(&[], 0, 0)]).unwrap();
        assert_eq!(out, [10, 0xFF, 0x00, 11, 0xFF, 12]);
    }

    #[test]
    fn source_slices_are_not_mutated() {
        let y = [1u8, 2, 3];
        let u = [4u8, 5];
        let v = [6u8];
        let _ = pack_yuv_planes([win(&y, 0, 3), win(&u, 0, 2), win(&v, 0, 1)]).unwrap();
        assert_eq!(y, [1, 2, 3]);
        assert_eq!(u, [4, 5]);
        assert_eq!(v, [6]);
    }
}
