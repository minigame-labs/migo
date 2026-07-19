//! Pointer input.
//!
//! One batched entry point mirroring the engine's `HostCommand::OnTouch`: the
//! host's points are validated and copied once into the fixed inline array that
//! `TouchData` already uses, which is the same path Android drives from
//! `platform/android/jni/inbound.rs`. No allocation on the input path.

use core::send_command_to_host;
use shared::protocol::host_cmd::{HostCommand, TouchData, TouchPoint, TouchType};

use crate::{
    abi::{
        guard, validate_header, MigoResult, VersionedHeader, MIGO_ERROR_INTERNAL,
        MIGO_ERROR_INVALID_ARGUMENT, MIGO_ERROR_INVALID_STATE, MIGO_ERROR_WOULD_BLOCK, MIGO_OK,
    },
    MigoSession,
};

/// `MIGO_TOUCH_*` from `include/migo/input.h`.
const MIGO_TOUCH_START: u32 = 0;
const MIGO_TOUCH_MOVE: u32 = 1;
const MIGO_TOUCH_END: u32 = 2;
const MIGO_TOUCH_CANCEL: u32 = 3;

/// `MIGO_TOUCH_FLAG_CHANGED` from `include/migo/input.h`. Mirrored for the tests
/// and for readers; the flags are the host's to set, not the ABI's.
#[allow(dead_code)]
const MIGO_TOUCH_FLAG_CHANGED: u32 = 1 << 0;

/// `MIGO_TOUCH_MAX_POINTS`; bounded by `TouchData::points`.
const MIGO_TOUCH_MAX_POINTS: u32 = 10;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MigoTouchPoint {
    pub(crate) id: u32,
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) pressure: f32,
    pub(crate) flags: u32,
}

// The C header asserts the same 20 bytes. Both assertions exist because a
// silent mismatch would corrupt every touch rather than fail loudly.
//
// Equal size is necessary but NOT sufficient: these two structs are copied
// bit-for-bit, and five 4-byte fields can be reordered without changing the
// size, which would turn every coordinate into another field's bits while both
// assertions still pass. `a_batch_arrives_with_every_field_in_place` checks the
// layout the copy actually depends on.
const _: () = assert!(size_of::<MigoTouchPoint>() == 20);
const _: () = assert!(size_of::<MigoTouchPoint>() == size_of::<TouchPoint>());

#[repr(C)]
pub struct MigoTouchEvent {
    pub(crate) header: VersionedHeader,
    pub(crate) touch_type: u32,
    pub(crate) point_count: u32,
    pub(crate) timestamp_ms: i64,
    pub(crate) points: *const MigoTouchPoint,
}

/// Translate a validated event envelope into the engine's `TouchData`.
///
/// Split out of the entry point so the reinterpreting copy is reachable from a
/// test without a live session or a surface: this is where a batch of several
/// pointers either arrives intact or turns to noise, and the size assertions
/// above cannot tell those apart.
///
/// Inlined so splitting it out costs nothing: `TouchData` carries the fixed
/// ten-point array, and returning one by value on the input path would
/// otherwise risk a ~200-byte copy per touch event that the previous
/// build-it-in-place shape did not pay.
///
/// # Safety
/// `event.points` must hold at least `event.point_count` entries.
#[inline]
unsafe fn to_touch_data(event: &MigoTouchEvent) -> Result<TouchData, MigoResult> {
    let touch_type = match event.touch_type {
        MIGO_TOUCH_START => TouchType::Start,
        MIGO_TOUCH_MOVE => TouchType::Move,
        MIGO_TOUCH_END => TouchType::End,
        MIGO_TOUCH_CANCEL => TouchType::Cancel,
        // Android coerces unknown platform action codes to Move because it
        // translates a fixed enum; a C host passing an undefined value has a
        // bug, and reinterpreting it would hide that.
        _ => return Err(MIGO_ERROR_INVALID_ARGUMENT),
    };
    if event.point_count == 0 || event.point_count > MIGO_TOUCH_MAX_POINTS {
        return Err(MIGO_ERROR_INVALID_ARGUMENT);
    }
    if event.points.is_null() {
        return Err(MIGO_ERROR_INVALID_ARGUMENT);
    }

    // One copy into the inline array, the same shape Android fills. Only the
    // first `count` entries are read; the rest keep their default so a stale
    // pointer from an earlier, longer batch can never be resurrected.
    let count = event.point_count as usize;
    let mut points = [TouchPoint::default(); 10];
    unsafe {
        std::ptr::copy_nonoverlapping(
            event.points as *const TouchPoint,
            points.as_mut_ptr(),
            count,
        );
    }

    Ok(TouchData {
        touch_type,
        count: count as u8,
        points,
        timestamp_ms: event.timestamp_ms,
    })
}

/// Deliver one touch event to the session's content.
///
/// # Safety
/// `session` must be live; `event` must point at a `MigoTouchEvent` whose
/// `points` array holds at least `point_count` entries. Both are borrowed for
/// the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn migo_session_send_touch(
    session: *mut MigoSession,
    event: *const MigoTouchEvent,
) -> MigoResult {
    guard("migo_session_send_touch", || {
        let Some(session) = (unsafe { session.as_ref() }) else {
            return MIGO_ERROR_INVALID_ARGUMENT;
        };
        if let Err(error) =
            unsafe { validate_header(event as *const VersionedHeader, size_of::<MigoTouchEvent>()) }
        {
            return error;
        }
        let event = unsafe { &*event };

        // Validated before the session lock so a malformed event is rejected on
        // its own terms rather than reporting whatever the session state is.
        let touch_data = match unsafe { to_touch_data(event) } {
            Ok(data) => data,
            Err(error) => return error,
        };

        let host = {
            let Ok(state) = session.state.lock() else {
                return MIGO_ERROR_INTERNAL;
            };
            if !state.attached {
                return MIGO_ERROR_INVALID_STATE;
            }
            match state.host {
                Some(host) => host,
                None => return MIGO_ERROR_INVALID_STATE,
            }
        };

        let command = HostCommand::OnTouch(Box::new(touch_data));

        // The host was present a moment ago under the lock, so a send failure
        // here is the queue being full rather than the session being gone. The
        // narrow race where the host shuts down in between reports WOULD_BLOCK
        // instead of INVALID_STATE; a host retrying then gets INVALID_STATE.
        match send_command_to_host(host, command) {
            Ok(()) => MIGO_OK,
            Err(error) => {
                tracing::debug!("migo_session_send_touch: not delivered: {error}");
                MIGO_ERROR_WOULD_BLOCK
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::{MIGO_ABI_VERSION_CURRENT, MIGO_ERROR_UNSUPPORTED_ABI};
    use crate::test_support::with_session;

    fn point() -> MigoTouchPoint {
        MigoTouchPoint {
            id: 0,
            x: 10.0,
            y: 20.0,
            pressure: 1.0,
            flags: MIGO_TOUCH_FLAG_CHANGED,
        }
    }

    fn event(points: &[MigoTouchPoint], count: u32) -> MigoTouchEvent {
        MigoTouchEvent {
            header: VersionedHeader {
                struct_size: size_of::<MigoTouchEvent>() as u32,
                abi_version: MIGO_ABI_VERSION_CURRENT,
            },
            touch_type: MIGO_TOUCH_MOVE,
            point_count: count,
            timestamp_ms: 1_234,
            points: points.as_ptr(),
        }
    }

    #[test]
    /// A batch of several pointers must arrive with every field where the
    /// engine expects it.
    ///
    /// The conversion reinterprets the C array as the engine's `TouchPoint`
    /// array, and the compile-time assertions above only compare sizes. Five
    /// 4-byte fields can be reordered on either side without changing the size,
    /// which would silently deliver one field's bits as another's -- a touch at
    /// the wrong coordinates, or a pointer id that is really a pressure value.
    /// Distinct values per field per point make a reorder or a stride error show
    /// up as a wrong number rather than as a passing size check.
    #[test]
    fn a_batch_arrives_with_every_field_in_place() {
        let points = [
            MigoTouchPoint { id: 7, x: 11.0, y: 22.0, pressure: 0.25, flags: 1 },
            MigoTouchPoint { id: 8, x: 33.0, y: 44.0, pressure: 0.50, flags: 0 },
            MigoTouchPoint { id: 9, x: 55.0, y: 66.0, pressure: 0.75, flags: 1 },
        ];
        let mut e = event(&points, points.len() as u32);
        e.touch_type = MIGO_TOUCH_MOVE;
        e.timestamp_ms = 1234;

        let data = unsafe { to_touch_data(&e) }.expect("a well-formed batch must convert");

        assert_eq!(data.touch_type, TouchType::Move);
        assert_eq!(data.count, 3);
        assert_eq!(data.timestamp_ms, 1234);
        for (index, expected) in points.iter().enumerate() {
            let got = data.points[index];
            assert_eq!(got.id, expected.id, "id of point {index}");
            assert_eq!(got.x, expected.x, "x of point {index}");
            assert_eq!(got.y, expected.y, "y of point {index}");
            assert_eq!(got.pressure, expected.pressure, "pressure of point {index}");
            assert_eq!(got.flags, expected.flags, "flags of point {index}");
        }
    }

    /// Points past the count belong to no one and must not be delivered.
    ///
    /// The destination array is fixed at ten entries and is reused for every
    /// batch shape, so an off-by-one here would hand the content a pointer the
    /// host never sent -- content that trusts `touches.length` would be fine,
    /// but anything walking the array would see a ghost finger.
    #[test]
    fn points_beyond_the_count_are_left_untouched() {
        let points = [
            MigoTouchPoint { id: 1, x: 1.0, y: 2.0, pressure: 1.0, flags: 1 },
            MigoTouchPoint { id: 2, x: 3.0, y: 4.0, pressure: 1.0, flags: 1 },
        ];
        // Announce one point while supplying two.
        let e = event(&points, 1);

        let data = unsafe { to_touch_data(&e) }.expect("a one-point batch must convert");

        assert_eq!(data.count, 1);
        assert_eq!(data.points[0].id, 1);
        // Compared field by field: `TouchPoint` is a shared-crate POD without
        // `PartialEq`, and deriving one there to serve a test in this crate
        // would widen a shared type's API for a local convenience.
        let ghost = data.points[1];
        let fresh = TouchPoint::default();
        assert_eq!(ghost.id, fresh.id, "the second slot must stay at its default");
        assert_eq!(ghost.x, fresh.x, "the second slot must stay at its default");
        assert_eq!(ghost.y, fresh.y, "the second slot must stay at its default");
        assert_eq!(ghost.flags, fresh.flags, "the second slot must stay at its default");
    }

    #[test]
    fn a_full_ten_point_batch_is_accepted() {
        // The documented maximum must convert, not just be under the rejection
        // threshold: this is the boundary a host reads off the header.
        let points: Vec<MigoTouchPoint> = (0..10)
            .map(|i| MigoTouchPoint {
                id: i,
                x: i as f32,
                y: (i * 2) as f32,
                pressure: 1.0,
                flags: MIGO_TOUCH_FLAG_CHANGED,
            })
            .collect();
        let e = event(&points, 10);

        let data = unsafe { to_touch_data(&e) }.expect("ten points is the documented maximum");

        assert_eq!(data.count, 10);
        assert_eq!(data.points[9].id, 9);
        assert_eq!(data.points[9].y, 18.0);
    }

    #[test]
    fn send_rejects_a_null_session() {
        let points = [point()];
        let e = event(&points, 1);
        assert_eq!(
            unsafe { migo_session_send_touch(std::ptr::null_mut(), &e) },
            MIGO_ERROR_INVALID_ARGUMENT
        );
    }

    #[test]
    fn send_rejects_a_null_event() {
        with_session("touch-null-event", |session| {
            assert_eq!(
                unsafe { migo_session_send_touch(session, std::ptr::null()) },
                MIGO_ERROR_INVALID_ARGUMENT
            );
        });
    }

    #[test]
    fn send_rejects_a_null_point_array() {
        with_session("touch-null-points", |session| {
            let points = [point()];
            let mut e = event(&points, 1);
            e.points = std::ptr::null();
            assert_eq!(
                unsafe { migo_session_send_touch(session, &e) },
                MIGO_ERROR_INVALID_ARGUMENT
            );
        });
    }

    #[test]
    fn send_rejects_a_struct_size_mismatch() {
        with_session("touch-size", |session| {
            let points = [point()];
            let mut e = event(&points, 1);
            e.header.struct_size = 8;
            assert_eq!(
                unsafe { migo_session_send_touch(session, &e) },
                MIGO_ERROR_INVALID_ARGUMENT
            );
        });
    }

    #[test]
    fn send_rejects_an_unknown_abi_version() {
        with_session("touch-abi", |session| {
            let points = [point()];
            let mut e = event(&points, 1);
            e.header.abi_version = 99;
            assert_eq!(
                unsafe { migo_session_send_touch(session, &e) },
                MIGO_ERROR_UNSUPPORTED_ABI
            );
        });
    }

    #[test]
    fn send_rejects_an_empty_or_overlong_batch() {
        // Zero has no meaning; 11 would overrun the engine's inline array.
        with_session("touch-count", |session| {
            let points = [point(); 10];
            assert_eq!(
                unsafe { migo_session_send_touch(session, &event(&points, 0)) },
                MIGO_ERROR_INVALID_ARGUMENT
            );
            assert_eq!(
                unsafe { migo_session_send_touch(session, &event(&points, 11)) },
                MIGO_ERROR_INVALID_ARGUMENT
            );
        });
    }

    #[test]
    fn send_rejects_an_undefined_touch_type() {
        with_session("touch-type", |session| {
            let points = [point()];
            let mut e = event(&points, 1);
            e.touch_type = 99;
            assert_eq!(
                unsafe { migo_session_send_touch(session, &e) },
                MIGO_ERROR_INVALID_ARGUMENT
            );
        });
    }

    #[test]
    fn send_without_an_attached_surface_reports_invalid_state() {
        // Reporting success would tell the host its event was delivered when
        // there is no content to deliver it to.
        with_session("touch-detached", |session| {
            let points = [point()];
            assert_eq!(
                unsafe { migo_session_send_touch(session, &event(&points, 1)) },
                MIGO_ERROR_INVALID_STATE
            );
        });
    }
}
