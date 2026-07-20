//! The soft keyboard's inbound half.
//!
//! One entry point carrying the wx keyboard model: the value a text event
//! carries is the field's whole current text, not the keystroke that changed
//! it, because that is what `HostCommand::OnKeyboardInput` means and what
//! content reads. A host that sends only the newly typed character leaves
//! content whose text never grows past one character.

use core::send_command_to_host;
use shared::protocol::host_cmd::HostCommand;

use crate::{
    abi::{
        guard, validate_header, MigoResult, VersionedHeader, MIGO_ERROR_INTERNAL,
        MIGO_ERROR_INVALID_ARGUMENT, MIGO_ERROR_INVALID_STATE, MIGO_ERROR_WOULD_BLOCK, MIGO_OK,
    },
    MigoSession,
};

/// `MIGO_KEYBOARD_EVENT_*` from `include/migo/input.h`.
const MIGO_KEYBOARD_EVENT_INPUT: u32 = 0;
const MIGO_KEYBOARD_EVENT_CONFIRM: u32 = 1;
const MIGO_KEYBOARD_EVENT_COMPLETE: u32 = 2;
const MIGO_KEYBOARD_EVENT_HEIGHT_CHANGE: u32 = 3;

#[repr(C)]
pub struct MigoKeyboardEvent {
    pub(crate) header: VersionedHeader,
    pub(crate) event_type: u32,
    pub(crate) value_length: u32,
    pub(crate) value_utf8: *const std::os::raw::c_char,
    pub(crate) height_css_px: f64,
}

/// Copy a length-delimited UTF-8 value from the host.
///
/// `abi::copy_utf8` cannot serve here: it derives the length by scanning for a
/// NUL, and this contract lets a host supply a buffer that has none. Scanning
/// one of those reads past the host's allocation.
///
/// # Safety
/// `value` must point at `length` readable bytes.
unsafe fn copy_utf8_with_length(
    value: *const std::os::raw::c_char,
    length: u32,
) -> Result<String, MigoResult> {
    if value.is_null() {
        return Err(MIGO_ERROR_INVALID_ARGUMENT);
    }
    let bytes = unsafe { std::slice::from_raw_parts(value as *const u8, length as usize) };
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| MIGO_ERROR_INVALID_ARGUMENT)
}

/// Translate a validated event into the engine's command.
///
/// Split out of the entry point so every rejection and every arm is reachable
/// from a test without a live session: this is where a host's event either
/// becomes the right engine command or the wrong one, and the entry point's
/// state checks cannot tell those apart.
///
/// # Safety
/// For a text event, `event.value_utf8` must point at `event.value_length`
/// readable bytes.
unsafe fn to_host_command(event: &MigoKeyboardEvent) -> Result<HostCommand, MigoResult> {
    match event.event_type {
        MIGO_KEYBOARD_EVENT_HEIGHT_CHANGE => {
            // NaN or a negative height would reach content's layout arithmetic
            // and there is no later event that repairs it.
            if !event.height_css_px.is_finite() || event.height_css_px < 0.0 {
                return Err(MIGO_ERROR_INVALID_ARGUMENT);
            }
            Ok(HostCommand::OnKeyboardHeightChange {
                height: event.height_css_px,
            })
        }
        kind => {
            let value = unsafe { copy_utf8_with_length(event.value_utf8, event.value_length) }?;
            match kind {
                MIGO_KEYBOARD_EVENT_INPUT => Ok(HostCommand::OnKeyboardInput { value }),
                MIGO_KEYBOARD_EVENT_CONFIRM => Ok(HostCommand::OnKeyboardConfirm { value }),
                MIGO_KEYBOARD_EVENT_COMPLETE => Ok(HostCommand::OnKeyboardComplete { value }),
                _ => Err(MIGO_ERROR_INVALID_ARGUMENT),
            }
        }
    }
}

/// Deliver one soft-keyboard event to the session's content.
///
/// # Safety
/// `session` must be live; `event` must point at a `MigoKeyboardEvent` whose
/// `value_utf8` holds at least `value_length` bytes. Both are borrowed for the
/// duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn migo_session_send_keyboard_event(
    session: *mut MigoSession,
    event: *const MigoKeyboardEvent,
) -> MigoResult {
    guard("migo_session_send_keyboard_event", || {
        let Some(session) = (unsafe { session.as_ref() }) else {
            return MIGO_ERROR_INVALID_ARGUMENT;
        };
        if let Err(error) = unsafe {
            validate_header(
                event as *const VersionedHeader,
                size_of::<MigoKeyboardEvent>(),
            )
        } {
            return error;
        }
        let event = unsafe { &*event };

        // Validated before the session lock so a malformed event is rejected on
        // its own terms rather than reporting whatever the session state is.
        let command = match unsafe { to_host_command(event) } {
            Ok(command) => command,
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

        // Dropping a COMPLETE would leave content believing the keyboard is
        // still open, and no later event corrects that -- the same reason the
        // touch path refuses to drop an END.
        match send_command_to_host(host, command) {
            Ok(()) => MIGO_OK,
            Err(error) => {
                tracing::debug!("migo_session_send_keyboard_event: not delivered: {error}");
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

    fn text_event(kind: u32, value: &str) -> MigoKeyboardEvent {
        MigoKeyboardEvent {
            header: VersionedHeader {
                struct_size: size_of::<MigoKeyboardEvent>() as u32,
                abi_version: MIGO_ABI_VERSION_CURRENT,
            },
            event_type: kind,
            value_length: value.len() as u32,
            value_utf8: value.as_ptr() as *const std::os::raw::c_char,
            height_css_px: 0.0,
        }
    }

    fn height_event(height: f64) -> MigoKeyboardEvent {
        MigoKeyboardEvent {
            header: VersionedHeader {
                struct_size: size_of::<MigoKeyboardEvent>() as u32,
                abi_version: MIGO_ABI_VERSION_CURRENT,
            },
            event_type: MIGO_KEYBOARD_EVENT_HEIGHT_CHANGE,
            value_length: 0,
            value_utf8: std::ptr::null(),
            height_css_px: height,
        }
    }

    /// Each event type must reach the matching engine command carrying its
    /// value. A swapped arm here would deliver a confirm as an input, which
    /// content sees as a field that never commits.
    #[test]
    fn every_event_type_reaches_its_own_command() {
        let cases = [
            (MIGO_KEYBOARD_EVENT_INPUT, "hello"),
            (MIGO_KEYBOARD_EVENT_CONFIRM, "hello"),
            (MIGO_KEYBOARD_EVENT_COMPLETE, "hello"),
        ];
        for (kind, value) in cases {
            let event = text_event(kind, value);
            let command = unsafe { to_host_command(&event) }.expect("well-formed");
            match (kind, command) {
                (MIGO_KEYBOARD_EVENT_INPUT, HostCommand::OnKeyboardInput { value: got }) => {
                    assert_eq!(got, value)
                }
                (MIGO_KEYBOARD_EVENT_CONFIRM, HostCommand::OnKeyboardConfirm { value: got }) => {
                    assert_eq!(got, value)
                }
                (MIGO_KEYBOARD_EVENT_COMPLETE, HostCommand::OnKeyboardComplete { value: got }) => {
                    assert_eq!(got, value)
                }
                (kind, other) => panic!("event type {kind} produced {other:?}"),
            }
        }
    }

    #[test]
    fn a_height_change_carries_its_height() {
        let event = height_event(320.5);
        match unsafe { to_host_command(&event) }.expect("well-formed") {
            HostCommand::OnKeyboardHeightChange { height } => assert_eq!(height, 320.5),
            other => panic!("expected a height change, got {other:?}"),
        }
    }

    /// Zero is how a host reports the keyboard closing, so it must convert.
    #[test]
    fn a_zero_height_is_valid_because_it_means_dismissed() {
        assert!(unsafe { to_host_command(&height_event(0.0)) }.is_ok());
    }

    /// NaN would reach content's layout arithmetic with no way back out.
    #[test]
    fn a_non_finite_or_negative_height_is_rejected() {
        for bad in [f64::NAN, f64::INFINITY, -1.0] {
            assert_eq!(
                unsafe { to_host_command(&height_event(bad)) }.err(),
                Some(MIGO_ERROR_INVALID_ARGUMENT),
                "height {bad} must be rejected"
            );
        }
    }

    /// An empty value is the content's field being cleared, which is a real
    /// event and not a malformed one.
    #[test]
    fn an_empty_value_is_valid() {
        let event = text_event(MIGO_KEYBOARD_EVENT_INPUT, "");
        match unsafe { to_host_command(&event) }.expect("well-formed") {
            HostCommand::OnKeyboardInput { value } => assert_eq!(value, ""),
            other => panic!("expected an input, got {other:?}"),
        }
    }

    /// The value is length-delimited, so a host may supply a buffer that is not
    /// NUL-terminated and one that has more bytes after the announced length.
    /// Reading past `value_length` would leak the host's adjacent memory into
    /// content.
    #[test]
    fn only_the_announced_length_is_read() {
        let backing = "abcXXXXX";
        let mut event = text_event(MIGO_KEYBOARD_EVENT_INPUT, backing);
        event.value_length = 3;
        match unsafe { to_host_command(&event) }.expect("well-formed") {
            HostCommand::OnKeyboardInput { value } => assert_eq!(value, "abc"),
            other => panic!("expected an input, got {other:?}"),
        }
    }

    /// Mangling what a user typed is worse than an error the host can see.
    #[test]
    fn an_invalid_utf8_value_is_rejected() {
        let bytes: [u8; 3] = [0x61, 0xff, 0x62];
        let mut event = text_event(MIGO_KEYBOARD_EVENT_INPUT, "abc");
        event.value_utf8 = bytes.as_ptr() as *const std::os::raw::c_char;
        event.value_length = 3;
        assert_eq!(
            unsafe { to_host_command(&event) }.err(),
            Some(MIGO_ERROR_INVALID_ARGUMENT)
        );
    }

    #[test]
    fn a_text_event_with_a_null_value_is_rejected() {
        let mut event = text_event(MIGO_KEYBOARD_EVENT_INPUT, "abc");
        event.value_utf8 = std::ptr::null();
        assert_eq!(
            unsafe { to_host_command(&event) }.err(),
            Some(MIGO_ERROR_INVALID_ARGUMENT)
        );
    }

    #[test]
    fn an_unknown_event_type_is_rejected() {
        let event = text_event(99, "abc");
        assert_eq!(
            unsafe { to_host_command(&event) }.err(),
            Some(MIGO_ERROR_INVALID_ARGUMENT)
        );
    }

    #[test]
    fn send_rejects_a_null_session() {
        let event = text_event(MIGO_KEYBOARD_EVENT_INPUT, "abc");
        assert_eq!(
            unsafe { migo_session_send_keyboard_event(std::ptr::null_mut(), &event) },
            MIGO_ERROR_INVALID_ARGUMENT
        );
    }

    #[test]
    fn send_rejects_a_null_event() {
        with_session("keyboard-null-event", |session| {
            assert_eq!(
                unsafe { migo_session_send_keyboard_event(session, std::ptr::null()) },
                MIGO_ERROR_INVALID_ARGUMENT
            );
        });
    }

    #[test]
    fn send_rejects_a_struct_size_mismatch() {
        with_session("keyboard-size", |session| {
            let mut event = text_event(MIGO_KEYBOARD_EVENT_INPUT, "abc");
            event.header.struct_size = 8;
            assert_eq!(
                unsafe { migo_session_send_keyboard_event(session, &event) },
                MIGO_ERROR_INVALID_ARGUMENT
            );
        });
    }

    #[test]
    fn send_rejects_an_unknown_abi_version() {
        with_session("keyboard-abi", |session| {
            let mut event = text_event(MIGO_KEYBOARD_EVENT_INPUT, "abc");
            event.header.abi_version = 99;
            assert_eq!(
                unsafe { migo_session_send_keyboard_event(session, &event) },
                MIGO_ERROR_UNSUPPORTED_ABI
            );
        });
    }

    /// Reporting success with nothing attached would tell the host its event
    /// was delivered when there is no content to deliver it to.
    #[test]
    fn send_without_an_attached_surface_reports_invalid_state() {
        with_session("keyboard-detached", |session| {
            let event = text_event(MIGO_KEYBOARD_EVENT_INPUT, "abc");
            assert_eq!(
                unsafe { migo_session_send_keyboard_event(session, &event) },
                MIGO_ERROR_INVALID_STATE
            );
        });
    }
}
