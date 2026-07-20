//! Gamepads, inbound.
//!
//! wx has no gamepad API, so the shape here is the Web one Migo replaces: a
//! host announces a pad, pushes samples while it is connected, and withdraws
//! it. Content polls `getGamepads()` -- the Web API is polled, not evented --
//! so a sample updates stored state rather than being delivered to a listener.

use core::send_command_to_host;
use shared::protocol::host_cmd::{
    GAMEPAD_MAX_AXES, GAMEPAD_MAX_BUTTONS, GamepadButtonState, GamepadState, HostCommand,
};

use crate::{
    MigoSession,
    abi::{
        MIGO_ERROR_INVALID_ARGUMENT, MIGO_ERROR_WOULD_BLOCK, MIGO_OK, MigoResult, VersionedHeader,
        copy_utf8, guard, validate_header,
    },
    keyboard::attached_host,
};

/// `MIGO_GAMEPAD_BUTTON_FLAG_*` from `include/migo/input.h`.
const MIGO_GAMEPAD_BUTTON_FLAG_PRESSED: u32 = 1 << 0;
const MIGO_GAMEPAD_BUTTON_FLAG_TOUCHED: u32 = 1 << 1;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MigoGamepadButton {
    pub(crate) flags: u32,
    pub(crate) value: f32,
}

const _: () = assert!(size_of::<MigoGamepadButton>() == 8);

#[repr(C)]
pub struct MigoGamepadInfo {
    pub(crate) header: VersionedHeader,
    pub(crate) index: u32,
    pub(crate) axis_count: u32,
    pub(crate) button_count: u32,
    pub(crate) reserved0: u32,
    pub(crate) id_utf8: *const std::os::raw::c_char,
    pub(crate) mapping_utf8: *const std::os::raw::c_char,
}

#[repr(C)]
pub struct MigoGamepadStateEvent {
    pub(crate) header: VersionedHeader,
    pub(crate) index: u32,
    pub(crate) axis_count: u32,
    pub(crate) button_count: u32,
    pub(crate) reserved0: u32,
    pub(crate) axes: *const f32,
    pub(crate) buttons: *const MigoGamepadButton,
    pub(crate) timestamp_ms: f64,
}

/// Translate a validated sample into the engine's state payload.
///
/// # Safety
/// `axes` and `buttons` must hold at least their announced counts.
unsafe fn to_gamepad_state(event: &MigoGamepadStateEvent) -> Result<GamepadState, MigoResult> {
    // Truncating here would silently drop a button content is watching, so an
    // over-long sample is refused: the counts a host sends must be the counts it
    // announced on connect, and a mismatch is a host bug worth surfacing.
    if event.axis_count as usize > GAMEPAD_MAX_AXES
        || event.button_count as usize > GAMEPAD_MAX_BUTTONS
    {
        return Err(MIGO_ERROR_INVALID_ARGUMENT);
    }
    if !event.timestamp_ms.is_finite() {
        return Err(MIGO_ERROR_INVALID_ARGUMENT);
    }
    if (event.axis_count > 0 && event.axes.is_null())
        || (event.button_count > 0 && event.buttons.is_null())
    {
        return Err(MIGO_ERROR_INVALID_ARGUMENT);
    }

    let axis_count = event.axis_count as usize;
    let button_count = event.button_count as usize;

    let mut axes = [0.0f32; GAMEPAD_MAX_AXES];
    if axis_count > 0 {
        let source = unsafe { std::slice::from_raw_parts(event.axes, axis_count) };
        // A NaN axis would reach content's movement arithmetic with no way back.
        if source.iter().any(|axis| !axis.is_finite()) {
            return Err(MIGO_ERROR_INVALID_ARGUMENT);
        }
        axes[..axis_count].copy_from_slice(source);
    }

    let mut buttons = [GamepadButtonState::default(); GAMEPAD_MAX_BUTTONS];
    if button_count > 0 {
        let source = unsafe { std::slice::from_raw_parts(event.buttons, button_count) };
        for (slot, raw) in buttons.iter_mut().zip(source) {
            if !raw.value.is_finite() {
                return Err(MIGO_ERROR_INVALID_ARGUMENT);
            }
            // `pressed` is carried rather than derived from `value`, because a
            // device chooses its own press threshold and content must not have
            // to guess one.
            slot.pressed = raw.flags & MIGO_GAMEPAD_BUTTON_FLAG_PRESSED != 0;
            slot.touched = raw.flags & MIGO_GAMEPAD_BUTTON_FLAG_TOUCHED != 0;
            slot.value = raw.value;
        }
    }

    Ok(GamepadState {
        index: event.index,
        axis_count: axis_count as u8,
        button_count: button_count as u8,
        axes,
        buttons,
        timestamp_ms: event.timestamp_ms,
    })
}

fn deliver(session: &MigoSession, command: HostCommand, entry: &str) -> MigoResult {
    let host = match attached_host(session) {
        Ok(host) => host,
        Err(error) => return error,
    };
    match send_command_to_host(host, command) {
        Ok(()) => MIGO_OK,
        Err(error) => {
            tracing::debug!("{entry}: not delivered: {error}");
            MIGO_ERROR_WOULD_BLOCK
        }
    }
}

/// Announce a gamepad, or withdraw one by passing `connected` as 0.
///
/// # Safety
/// `session` must be live; `info` must point at a `MigoGamepadInfo` whose
/// strings are NUL-terminated. Both are borrowed for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn migo_session_set_gamepad_connected(
    session: *mut MigoSession,
    info: *const MigoGamepadInfo,
    connected: u32,
) -> MigoResult {
    guard("migo_session_set_gamepad_connected", || {
        let Some(session) = (unsafe { session.as_ref() }) else {
            return MIGO_ERROR_INVALID_ARGUMENT;
        };
        if let Err(error) =
            unsafe { validate_header(info as *const VersionedHeader, size_of::<MigoGamepadInfo>()) }
        {
            return error;
        }
        let info = unsafe { &*info };

        let command = if connected == 0 {
            HostCommand::OnGamepadDisconnected { index: info.index }
        } else {
            if info.axis_count as usize > GAMEPAD_MAX_AXES
                || info.button_count as usize > GAMEPAD_MAX_BUTTONS
            {
                return MIGO_ERROR_INVALID_ARGUMENT;
            }
            let (id, mapping) = match (unsafe { copy_utf8(info.id_utf8) }, unsafe {
                copy_utf8(info.mapping_utf8)
            }) {
                (Ok(id), Ok(mapping)) => (id, mapping),
                _ => return MIGO_ERROR_INVALID_ARGUMENT,
            };
            HostCommand::OnGamepadConnected {
                index: info.index,
                id,
                mapping,
                axis_count: info.axis_count as u8,
                button_count: info.button_count as u8,
            }
        };
        deliver(session, command, "migo_session_set_gamepad_connected")
    })
}

/// Push one sample of a connected gamepad's axes and buttons.
///
/// # Safety
/// `session` must be live; `event`'s `axes` and `buttons` must each hold at
/// least their announced counts. All are borrowed for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn migo_session_send_gamepad_state(
    session: *mut MigoSession,
    event: *const MigoGamepadStateEvent,
) -> MigoResult {
    guard("migo_session_send_gamepad_state", || {
        let Some(session) = (unsafe { session.as_ref() }) else {
            return MIGO_ERROR_INVALID_ARGUMENT;
        };
        if let Err(error) = unsafe {
            validate_header(
                event as *const VersionedHeader,
                size_of::<MigoGamepadStateEvent>(),
            )
        } {
            return error;
        }
        let event = unsafe { &*event };

        let state = match unsafe { to_gamepad_state(event) } {
            Ok(state) => state,
            Err(error) => return error,
        };
        // A dropped sample is harmless: the API is polled, so the next one
        // replaces it wholesale. Reported anyway, because a host seeing this
        // every frame is sampling faster than the engine drains.
        deliver(
            session,
            HostCommand::OnGamepadState(Box::new(state)),
            "migo_session_send_gamepad_state",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::MIGO_ABI_VERSION_CURRENT;

    fn button(pressed: bool, touched: bool, value: f32) -> MigoGamepadButton {
        let mut flags = 0;
        if pressed {
            flags |= MIGO_GAMEPAD_BUTTON_FLAG_PRESSED;
        }
        if touched {
            flags |= MIGO_GAMEPAD_BUTTON_FLAG_TOUCHED;
        }
        MigoGamepadButton { flags, value }
    }

    fn state_event(axes: &[f32], buttons: &[MigoGamepadButton]) -> MigoGamepadStateEvent {
        MigoGamepadStateEvent {
            header: VersionedHeader {
                struct_size: size_of::<MigoGamepadStateEvent>() as u32,
                abi_version: MIGO_ABI_VERSION_CURRENT,
            },
            index: 0,
            axis_count: axes.len() as u32,
            button_count: buttons.len() as u32,
            reserved0: 0,
            axes: axes.as_ptr(),
            buttons: buttons.as_ptr(),
            timestamp_ms: 16.0,
        }
    }

    /// Every axis and every button flag must land where content reads it. A
    /// swapped flag bit turns a resting trigger into a held one, which content
    /// sees as a button nobody pressed.
    #[test]
    fn a_sample_arrives_with_every_axis_and_button_in_place() {
        let axes = [-1.0f32, 0.0, 0.5, 1.0];
        let buttons = [
            button(true, true, 1.0),
            button(false, true, 0.25),
            button(false, false, 0.0),
        ];
        let state =
            unsafe { to_gamepad_state(&state_event(&axes, &buttons)) }.expect("well-formed");

        assert_eq!(state.axis_count, 4);
        assert_eq!(state.button_count, 3);
        for (index, expected) in axes.iter().enumerate() {
            assert_eq!(state.axes[index], *expected, "axis {index}");
        }
        assert!(state.buttons[0].pressed && state.buttons[0].touched);
        assert_eq!(state.buttons[0].value, 1.0);
        assert!(!state.buttons[1].pressed && state.buttons[1].touched);
        assert_eq!(state.buttons[1].value, 0.25);
        assert!(!state.buttons[2].pressed && !state.buttons[2].touched);
    }

    /// `pressed` is carried, not derived: a device picks its own threshold, and
    /// a trigger held at 0.25 may or may not count as pressed depending on the
    /// pad. Deriving it here would overrule the device.
    #[test]
    fn pressed_is_independent_of_value() {
        let buttons = [button(true, false, 0.0), button(false, false, 0.9)];
        let state = unsafe { to_gamepad_state(&state_event(&[], &buttons)) }.expect("well-formed");
        assert!(state.buttons[0].pressed, "a pressed button at rest value");
        assert!(
            !state.buttons[1].pressed,
            "an unpressed button near full travel"
        );
    }

    /// Slots past the announced counts belong to nobody.
    #[test]
    fn entries_beyond_the_counts_keep_their_defaults() {
        let axes = [1.0f32];
        let buttons = [button(true, true, 1.0)];
        let state =
            unsafe { to_gamepad_state(&state_event(&axes, &buttons)) }.expect("well-formed");
        assert_eq!(state.axes[1], 0.0);
        assert_eq!(state.buttons[1], GamepadButtonState::default());
    }

    /// Truncating would silently drop a button content is watching, so a sample
    /// bigger than the runtime carries is refused rather than shortened.
    #[test]
    fn an_oversized_sample_is_rejected_rather_than_truncated() {
        let axes = [0.0f32; GAMEPAD_MAX_AXES + 1];
        assert_eq!(
            unsafe { to_gamepad_state(&state_event(&axes, &[])) }.err(),
            Some(MIGO_ERROR_INVALID_ARGUMENT)
        );
        let buttons = [button(false, false, 0.0); GAMEPAD_MAX_BUTTONS + 1];
        assert_eq!(
            unsafe { to_gamepad_state(&state_event(&[], &buttons)) }.err(),
            Some(MIGO_ERROR_INVALID_ARGUMENT)
        );
    }

    /// A NaN axis would reach content's movement arithmetic with no way back.
    #[test]
    fn a_non_finite_axis_value_or_timestamp_is_rejected() {
        for bad in [f32::NAN, f32::INFINITY] {
            let axes = [bad];
            assert_eq!(
                unsafe { to_gamepad_state(&state_event(&axes, &[])) }.err(),
                Some(MIGO_ERROR_INVALID_ARGUMENT)
            );
        }
        let mut event = state_event(&[], &[]);
        event.timestamp_ms = f64::NAN;
        assert_eq!(
            unsafe { to_gamepad_state(&event) }.err(),
            Some(MIGO_ERROR_INVALID_ARGUMENT)
        );
    }

    /// A pad with no axes and no buttons is degenerate but not malformed, and
    /// the null pointers that come with it must not be dereferenced.
    #[test]
    fn an_empty_sample_with_null_arrays_is_accepted() {
        let mut event = state_event(&[], &[]);
        event.axes = std::ptr::null();
        event.buttons = std::ptr::null();
        let state = unsafe { to_gamepad_state(&event) }.expect("an empty sample is well-formed");
        assert_eq!(state.axis_count, 0);
        assert_eq!(state.button_count, 0);
    }

    #[test]
    fn a_sample_announcing_entries_it_does_not_supply_is_rejected() {
        let mut event = state_event(&[1.0], &[]);
        event.axes = std::ptr::null();
        assert_eq!(
            unsafe { to_gamepad_state(&event) }.err(),
            Some(MIGO_ERROR_INVALID_ARGUMENT)
        );
    }
}
