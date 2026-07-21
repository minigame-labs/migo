//! Pointer input.
//!
//! One batched entry point mirroring the engine's `HostCommand::OnTouch`: the
//! host's points are validated and copied into fixed inline storage before any
//! Session state is touched. Direct ingress then transfers that bounded value
//! through the per-Host payload pool, so steady-state touch delivery performs
//! no heap allocation and takes no Session mutex.

use std::mem::{align_of, offset_of, size_of};

use shared::protocol::host_cmd::{TouchData, TouchPoint, TouchType};

use crate::panic_barrier::guard;
use crate::{MigoSession, map_ingress_result, pin_session};
use migo_capi_abi::{MIGO_ERROR_INVALID_ARGUMENT, MigoResult};

use migo_capi_abi::input::{
    MIGO_TOUCH_CANCEL, MIGO_TOUCH_END, MIGO_TOUCH_MOVE, MIGO_TOUCH_START, ValidatedTouchEvent,
};
pub use migo_capi_abi::input::{MigoTouchEvent, MigoTouchPoint};

#[cfg(test)]
use migo_capi_abi::MIGO_ERROR_INVALID_STATE;
#[cfg(test)]
use migo_capi_abi::input::MIGO_TOUCH_FLAG_CHANGED;

// The C header asserts the same 20 bytes. Both assertions exist because a
// silent mismatch would corrupt every touch rather than fail loudly.
//
// Equal size is necessary but NOT sufficient: these two structs are copied
// bit-for-bit, and five 4-byte fields can be reordered without changing the
// size, which would turn every coordinate into another field's bits while both
// assertions still pass. `a_batch_arrives_with_every_field_in_place` checks the
// layout the copy actually depends on.
const _: () = assert!(size_of::<MigoTouchPoint>() == size_of::<TouchPoint>());
const _: () = assert!(align_of::<MigoTouchPoint>() == align_of::<TouchPoint>());
const _: () = assert!(offset_of!(MigoTouchPoint, id) == offset_of!(TouchPoint, id));
const _: () = assert!(offset_of!(MigoTouchPoint, x) == offset_of!(TouchPoint, x));
const _: () = assert!(offset_of!(MigoTouchPoint, y) == offset_of!(TouchPoint, y));
const _: () = assert!(offset_of!(MigoTouchPoint, pressure) == offset_of!(TouchPoint, pressure));
const _: () = assert!(offset_of!(MigoTouchPoint, flags) == offset_of!(TouchPoint, flags));

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
#[inline]
fn validated_to_touch_data(event: ValidatedTouchEvent) -> TouchData {
    let (touch_type, count, points, timestamp_ms) = event.into_parts();
    let touch_type = match touch_type {
        MIGO_TOUCH_START => TouchType::Start,
        MIGO_TOUCH_MOVE => TouchType::Move,
        MIGO_TOUCH_END => TouchType::End,
        MIGO_TOUCH_CANCEL => TouchType::Cancel,
        _ => unreachable!("validated touch kind"),
    };

    // Both records are repr(C), have identical field offsets/alignment, and the
    // ABI validator has already copied/checked the complete array. Moving the
    // array preserves the one caller-memory copy instead of mapping 10 points.
    let points = unsafe { std::mem::transmute::<[MigoTouchPoint; 10], [TouchPoint; 10]>(points) };
    TouchData {
        touch_type,
        count,
        points,
        timestamp_ms,
    }
}

/// Testable composition of boundary validation and engine conversion.
///
/// # Safety
/// `event.points` must hold at least `event.point_count` entries.
#[inline]
unsafe fn to_touch_data(event: &MigoTouchEvent) -> Result<TouchData, MigoResult> {
    unsafe { event.validate() }.map(validated_to_touch_data)
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
        let session = match unsafe { pin_session(session) } {
            Ok(session) => session,
            Err(error) => return error,
        };
        // Validated before the session lock so a malformed event is rejected on
        // its own terms rather than reporting whatever the session state is.
        let touch_data = match unsafe { MigoTouchEvent::parse(event) } {
            Ok(event) => validated_to_touch_data(event),
            Err(error) => return error,
        };

        let ingress = match session.active_ingress() {
            Ok(ingress) => ingress,
            Err(error) => return error,
        };
        map_ingress_result(
            "migo_session_send_touch",
            ingress.try_send_touch(touch_data),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::with_session;
    use migo_capi_abi::{MIGO_ABI_VERSION_CURRENT, MIGO_ERROR_UNSUPPORTED_ABI, VersionedHeader};

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
            MigoTouchPoint {
                id: 7,
                x: 11.0,
                y: 22.0,
                pressure: 0.25,
                flags: 1,
            },
            MigoTouchPoint {
                id: 8,
                x: 33.0,
                y: 44.0,
                pressure: 0.50,
                flags: 0,
            },
            MigoTouchPoint {
                id: 9,
                x: 55.0,
                y: 66.0,
                pressure: 0.75,
                flags: 1,
            },
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
            MigoTouchPoint {
                id: 1,
                x: 1.0,
                y: 2.0,
                pressure: 1.0,
                flags: 1,
            },
            MigoTouchPoint {
                id: 2,
                x: 3.0,
                y: 4.0,
                pressure: 1.0,
                flags: 1,
            },
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
        assert_eq!(
            ghost.id, fresh.id,
            "the second slot must stay at its default"
        );
        assert_eq!(ghost.x, fresh.x, "the second slot must stay at its default");
        assert_eq!(ghost.y, fresh.y, "the second slot must stay at its default");
        assert_eq!(
            ghost.flags, fresh.flags,
            "the second slot must stay at its default"
        );
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
