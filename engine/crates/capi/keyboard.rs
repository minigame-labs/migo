//! Keyboard input, inbound.
//!
//! Two capabilities that share a noun and nothing else. The soft keyboard is
//! text: the host owns an IME and reports the field's whole current value, not
//! the keystroke that changed it -- a host that sends only the newly typed
//! character leaves content whose text never grows past one character. A
//! physical key is a discrete press of an identified key. Content reads the two
//! through different listeners.

use shared::protocol::host_cmd::HostCommand;

use crate::{
    MigoSession,
    abi::{MIGO_ERROR_INVALID_ARGUMENT, MigoResult, guard},
    map_ingress_result, pin_session,
};

use migo_capi_abi::input::{
    MIGO_COMPOSITION_EVENT_END, MIGO_COMPOSITION_EVENT_START, MIGO_COMPOSITION_EVENT_UPDATE,
    MIGO_KEY_EVENT_DOWN, MIGO_KEY_EVENT_UP, MIGO_KEYBOARD_EVENT_COMPLETE,
    MIGO_KEYBOARD_EVENT_CONFIRM, MIGO_KEYBOARD_EVENT_HEIGHT_CHANGE, MIGO_KEYBOARD_EVENT_INPUT,
    ValidatedCompositionEvent, ValidatedKeyEvent, ValidatedKeyboardEvent,
};
pub use migo_capi_abi::input::{MigoCompositionEvent, MigoKeyEvent, MigoKeyboardEvent};

#[cfg(test)]
use crate::abi::MIGO_ERROR_INVALID_STATE;

/// Translate a validated event into the engine's command.
///
/// Split out of the entry point so every rejection and every arm is reachable
/// from a test without a live session: this is where a host's event either
/// becomes the right engine command or the wrong one, and the entry point's
/// state checks cannot tell those apart.
///
fn validated_keyboard_to_command(event: ValidatedKeyboardEvent) -> HostCommand {
    match event {
        ValidatedKeyboardEvent::Input(value) => HostCommand::OnKeyboardInput { value },
        ValidatedKeyboardEvent::Confirm(value) => HostCommand::OnKeyboardConfirm { value },
        ValidatedKeyboardEvent::Complete(value) => HostCommand::OnKeyboardComplete { value },
        ValidatedKeyboardEvent::HeightChange(height) => {
            HostCommand::OnKeyboardHeightChange { height }
        }
    }
}

/// Testable composition of pure boundary validation and command conversion.
unsafe fn to_host_command(event: &MigoKeyboardEvent) -> Result<HostCommand, MigoResult> {
    unsafe { event.validate() }.map(validated_keyboard_to_command)
}

/// Translate a validated composition event into the engine's command.
///
fn validated_composition_to_command(event: ValidatedCompositionEvent) -> HostCommand {
    match event {
        ValidatedCompositionEvent::Start(data) => HostCommand::OnCompositionStart { data },
        ValidatedCompositionEvent::Update(data) => HostCommand::OnCompositionUpdate { data },
        ValidatedCompositionEvent::End(data) => HostCommand::OnCompositionEnd { data },
    }
}

unsafe fn to_composition_command(event: &MigoCompositionEvent) -> Result<HostCommand, MigoResult> {
    unsafe { event.validate() }.map(validated_composition_to_command)
}

/// Deliver one IME composition event to the session's content.
///
/// # Safety
/// `session` must be live; `event` must point at a `MigoCompositionEvent` whose
/// `data_utf8` holds at least `data_length` bytes. Both are borrowed for the
/// duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn migo_session_send_composition_event(
    session: *mut MigoSession,
    event: *const MigoCompositionEvent,
) -> MigoResult {
    guard("migo_session_send_composition_event", || {
        let session = match unsafe { pin_session(session) } {
            Ok(session) => session,
            Err(error) => return error,
        };
        let command = match unsafe { MigoCompositionEvent::parse(event) } {
            Ok(event) => validated_composition_to_command(event),
            Err(error) => return error,
        };
        let ingress = match session.active_ingress() {
            Ok(ingress) => ingress,
            Err(error) => return error,
        };

        // A dropped END leaves content drawing a preedit that will never be
        // cleared, and no later event corrects it.
        map_ingress_result(
            "migo_session_send_composition_event",
            ingress.try_send(command),
        )
    })
}

/// Translate a validated physical-key event into the engine's command.
///
/// # Safety
/// `key_utf8` and `code_utf8` must each point at their announced number of
/// readable bytes.
fn validated_key_to_command(event: ValidatedKeyEvent) -> HostCommand {
    match event {
        ValidatedKeyEvent::Down {
            key,
            code,
            timestamp_ms,
        } => HostCommand::OnKeyDown {
            key,
            code,
            timestamp_ms,
        },
        ValidatedKeyEvent::Up {
            key,
            code,
            timestamp_ms,
        } => HostCommand::OnKeyUp {
            key,
            code,
            timestamp_ms,
        },
    }
}

unsafe fn to_key_command(event: &MigoKeyEvent) -> Result<HostCommand, MigoResult> {
    unsafe { event.validate() }.map(validated_key_to_command)
}

/// Deliver one physical key press or release to the session's content.
///
/// # Safety
/// `session` must be live; `event` must point at a `MigoKeyEvent` whose two
/// strings each hold at least their announced length. All are borrowed for the
/// duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn migo_session_send_key_event(
    session: *mut MigoSession,
    event: *const MigoKeyEvent,
) -> MigoResult {
    guard("migo_session_send_key_event", || {
        let session = match unsafe { pin_session(session) } {
            Ok(session) => session,
            Err(error) => return error,
        };
        let command = match unsafe { MigoKeyEvent::parse(event) } {
            Ok(event) => validated_key_to_command(event),
            Err(error) => return error,
        };
        let ingress = match session.active_ingress() {
            Ok(ingress) => ingress,
            Err(error) => return error,
        };

        // A dropped UP leaves content believing the key is still held, and no
        // later event corrects it.
        map_ingress_result("migo_session_send_key_event", ingress.try_send(command))
    })
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
        let session = match unsafe { pin_session(session) } {
            Ok(session) => session,
            Err(error) => return error,
        };
        // Validated before the session lock so a malformed event is rejected on
        // its own terms rather than reporting whatever the session state is.
        let command = match unsafe { MigoKeyboardEvent::parse(event) } {
            Ok(event) => validated_keyboard_to_command(event),
            Err(error) => return error,
        };

        let ingress = match session.active_ingress() {
            Ok(ingress) => ingress,
            Err(error) => return error,
        };

        // Dropping a COMPLETE would leave content believing the keyboard is
        // still open, and no later event corrects that -- the same reason the
        // touch path refuses to drop an END.
        map_ingress_result(
            "migo_session_send_keyboard_event",
            ingress.try_send(command),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::{MIGO_ABI_VERSION_CURRENT, MIGO_ERROR_UNSUPPORTED_ABI, VersionedHeader};
    use crate::test_support::with_session;
    use std::mem::size_of;

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

    fn composition_event(kind: u32, data: &str) -> MigoCompositionEvent {
        MigoCompositionEvent {
            header: VersionedHeader {
                struct_size: size_of::<MigoCompositionEvent>() as u32,
                abi_version: MIGO_ABI_VERSION_CURRENT,
            },
            event_type: kind,
            data_length: data.len() as u32,
            data_utf8: data.as_ptr() as *const std::os::raw::c_char,
        }
    }

    /// Each composition event must reach its own command. Content draws the
    /// preedit on update and clears it on end, so a swapped arm leaves text on
    /// screen that the user already committed or cancelled.
    #[test]
    fn every_composition_type_reaches_its_own_command() {
        let cases = [
            (MIGO_COMPOSITION_EVENT_START, ""),
            (MIGO_COMPOSITION_EVENT_UPDATE, "nihao"),
            (MIGO_COMPOSITION_EVENT_END, "\u{4f60}\u{597d}"),
        ];
        for (kind, data) in cases {
            let event = composition_event(kind, data);
            match (
                kind,
                unsafe { to_composition_command(&event) }.expect("well-formed"),
            ) {
                (MIGO_COMPOSITION_EVENT_START, HostCommand::OnCompositionStart { data: got }) => {
                    assert_eq!(got, data)
                }
                (MIGO_COMPOSITION_EVENT_UPDATE, HostCommand::OnCompositionUpdate { data: got }) => {
                    assert_eq!(got, data)
                }
                (MIGO_COMPOSITION_EVENT_END, HostCommand::OnCompositionEnd { data: got }) => {
                    assert_eq!(got, data)
                }
                (kind, other) => panic!("composition type {kind} produced {other:?}"),
            }
        }
    }

    /// Multi-byte preedit text is the whole point of composition, so a length
    /// that splits a character must be refused rather than delivered mangled.
    #[test]
    fn a_composition_length_that_splits_a_character_is_rejected() {
        let data = "\u{4f60}\u{597d}";
        let mut event = composition_event(MIGO_COMPOSITION_EVENT_UPDATE, data);
        event.data_length = 1; // half of the first code point
        assert_eq!(
            unsafe { to_composition_command(&event) }.err(),
            Some(MIGO_ERROR_INVALID_ARGUMENT)
        );
    }

    /// An empty end is a cancelled composition, which content must still see so
    /// it can clear the preedit it has been drawing.
    #[test]
    fn an_empty_composition_end_is_valid_because_it_means_cancelled() {
        let event = composition_event(MIGO_COMPOSITION_EVENT_END, "");
        match unsafe { to_composition_command(&event) }.expect("well-formed") {
            HostCommand::OnCompositionEnd { data } => assert_eq!(data, ""),
            other => panic!("expected a composition end, got {other:?}"),
        }
    }

    #[test]
    fn an_unknown_composition_type_is_rejected() {
        assert_eq!(
            unsafe { to_composition_command(&composition_event(99, "x")) }.err(),
            Some(MIGO_ERROR_INVALID_ARGUMENT)
        );
    }

    #[test]
    fn send_composition_without_an_attached_surface_reports_invalid_state() {
        with_session("composition-detached", |session| {
            let event = composition_event(MIGO_COMPOSITION_EVENT_UPDATE, "ni");
            assert_eq!(
                unsafe { migo_session_send_composition_event(session, &event) },
                MIGO_ERROR_INVALID_STATE
            );
        });
    }

    fn key_event(kind: u32, key: &str, code: &str) -> MigoKeyEvent {
        MigoKeyEvent {
            header: VersionedHeader {
                struct_size: size_of::<MigoKeyEvent>() as u32,
                abi_version: MIGO_ABI_VERSION_CURRENT,
            },
            event_type: kind,
            key_length: key.len() as u32,
            key_utf8: key.as_ptr() as *const std::os::raw::c_char,
            code_utf8: code.as_ptr() as *const std::os::raw::c_char,
            code_length: code.len() as u32,
            reserved0: 0,
            timestamp_ms: 1234.0,
        }
    }

    /// `key` and `code` are different values and must not be swapped: a host
    /// that sends `code` in both leaves content reading "KeyA" as typed text.
    /// Distinct values on each side are what makes a swap show up here rather
    /// than in a game.
    #[test]
    fn down_and_up_carry_key_and_code_in_the_right_places() {
        for (kind, expect_down) in [(MIGO_KEY_EVENT_DOWN, true), (MIGO_KEY_EVENT_UP, false)] {
            let event = key_event(kind, "a", "KeyA");
            match unsafe { to_key_command(&event) }.expect("well-formed") {
                HostCommand::OnKeyDown {
                    key,
                    code,
                    timestamp_ms,
                } => {
                    assert!(expect_down, "a down arm produced an up command");
                    assert_eq!(key, "a");
                    assert_eq!(code, "KeyA");
                    assert_eq!(timestamp_ms, 1234.0);
                }
                HostCommand::OnKeyUp { key, code, .. } => {
                    assert!(!expect_down, "an up arm produced a down command");
                    assert_eq!(key, "a");
                    assert_eq!(code, "KeyA");
                }
                other => panic!("expected a key command, got {other:?}"),
            }
        }
    }

    /// A dead key produces no text, so an empty `key` is a real event.
    #[test]
    fn an_empty_key_is_valid_because_some_keys_produce_no_text() {
        let event = key_event(MIGO_KEY_EVENT_DOWN, "", "Backquote");
        match unsafe { to_key_command(&event) }.expect("well-formed") {
            HostCommand::OnKeyDown { key, code, .. } => {
                assert_eq!(key, "");
                assert_eq!(code, "Backquote");
            }
            other => panic!("expected a key down, got {other:?}"),
        }
    }

    /// `code` always identifies a physical key, so an empty one means the host
    /// did not fill it in -- not that the key has no identity.
    #[test]
    fn an_empty_code_is_rejected() {
        let event = key_event(MIGO_KEY_EVENT_DOWN, "a", "");
        assert_eq!(
            unsafe { to_key_command(&event) }.err(),
            Some(MIGO_ERROR_INVALID_ARGUMENT)
        );
    }

    #[test]
    fn an_unknown_key_event_type_is_rejected() {
        assert_eq!(
            unsafe { to_key_command(&key_event(99, "a", "KeyA")) }.err(),
            Some(MIGO_ERROR_INVALID_ARGUMENT)
        );
    }

    #[test]
    fn a_non_finite_key_timestamp_is_rejected() {
        let mut event = key_event(MIGO_KEY_EVENT_DOWN, "a", "KeyA");
        event.timestamp_ms = f64::NAN;
        assert_eq!(
            unsafe { to_key_command(&event) }.err(),
            Some(MIGO_ERROR_INVALID_ARGUMENT)
        );
    }

    #[test]
    fn a_key_event_with_a_null_string_is_rejected() {
        let mut null_key = key_event(MIGO_KEY_EVENT_DOWN, "a", "KeyA");
        null_key.key_utf8 = std::ptr::null();
        assert_eq!(
            unsafe { to_key_command(&null_key) }.err(),
            Some(MIGO_ERROR_INVALID_ARGUMENT)
        );

        let mut null_code = key_event(MIGO_KEY_EVENT_DOWN, "a", "KeyA");
        null_code.code_utf8 = std::ptr::null();
        assert_eq!(
            unsafe { to_key_command(&null_code) }.err(),
            Some(MIGO_ERROR_INVALID_ARGUMENT)
        );
    }

    /// Only the announced lengths are read: a host may hold either string in a
    /// larger buffer with no NUL, and reading past would leak its memory into
    /// content.
    #[test]
    fn only_the_announced_key_lengths_are_read() {
        let mut event = key_event(MIGO_KEY_EVENT_DOWN, "abcdef", "KeyAxxxx");
        event.key_length = 1;
        event.code_length = 4;
        match unsafe { to_key_command(&event) }.expect("well-formed") {
            HostCommand::OnKeyDown { key, code, .. } => {
                assert_eq!(key, "a");
                assert_eq!(code, "KeyA");
            }
            other => panic!("expected a key down, got {other:?}"),
        }
    }

    #[test]
    fn send_key_without_an_attached_surface_reports_invalid_state() {
        with_session("key-detached", |session| {
            let event = key_event(MIGO_KEY_EVENT_DOWN, "a", "KeyA");
            assert_eq!(
                unsafe { migo_session_send_key_event(session, &event) },
                MIGO_ERROR_INVALID_STATE
            );
        });
    }

    #[test]
    fn send_key_rejects_a_null_session_and_a_null_event() {
        let event = key_event(MIGO_KEY_EVENT_DOWN, "a", "KeyA");
        assert_eq!(
            unsafe { migo_session_send_key_event(std::ptr::null_mut(), &event) },
            MIGO_ERROR_INVALID_ARGUMENT
        );
        with_session("key-null-event", |session| {
            assert_eq!(
                unsafe { migo_session_send_key_event(session, std::ptr::null()) },
                MIGO_ERROR_INVALID_ARGUMENT
            );
        });
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
