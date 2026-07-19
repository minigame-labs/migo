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

        let touch_type = match event.touch_type {
            MIGO_TOUCH_START => TouchType::Start,
            MIGO_TOUCH_MOVE => TouchType::Move,
            MIGO_TOUCH_END => TouchType::End,
            MIGO_TOUCH_CANCEL => TouchType::Cancel,
            // Android coerces unknown platform action codes to Move because it
            // translates a fixed enum; a C host passing an undefined value has a
            // bug, and reinterpreting it would hide that.
            _ => return MIGO_ERROR_INVALID_ARGUMENT,
        };
        if event.point_count == 0 || event.point_count > MIGO_TOUCH_MAX_POINTS {
            return MIGO_ERROR_INVALID_ARGUMENT;
        }
        if event.points.is_null() {
            return MIGO_ERROR_INVALID_ARGUMENT;
        }

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

        // One copy into the inline array, the same shape Android fills.
        let count = event.point_count as usize;
        let mut points = [TouchPoint::default(); 10];
        unsafe {
            std::ptr::copy_nonoverlapping(
                event.points as *const TouchPoint,
                points.as_mut_ptr(),
                count,
            );
        }

        let command = HostCommand::OnTouch(Box::new(TouchData {
            touch_type,
            count: count as u8,
            points,
            timestamp_ms: event.timestamp_ms,
        }));

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
