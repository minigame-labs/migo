use std::collections::HashMap;

use shared::protocol::host_cmd::{TouchData, TouchPoint, TouchType};

const TOUCH_CHANGED: u32 = 1;
const TOUCH_REMOVED: u32 = 2;
const POINTER_BUTTON_SLOTS: usize = 32;

#[derive(Debug)]
pub(crate) enum InputRetraction {
    TouchCancel(TouchData),
    MouseUp {
        x: f32,
        y: f32,
        button: u32,
        timestamp_ms: f64,
    },
    KeyUp {
        key: String,
        code: String,
        timestamp_ms: f64,
        modifiers: u32,
    },
    CompositionEnd,
}

struct KeyState {
    key: String,
    timestamp_ms: f64,
    modifiers: u32,
}

#[derive(Clone, Copy)]
struct PointerPosition {
    x: f32,
    y: f32,
    timestamp_ms: f64,
}

#[derive(Default)]
pub(crate) struct InputState {
    active_touches: [Option<TouchPoint>; 10],
    last_touch_timestamp_ms: i64,
    pointer_buttons: [bool; POINTER_BUTTON_SLOTS],
    pointer_position: Option<PointerPosition>,
    keys: HashMap<String, KeyState>,
    composition_active: bool,
}

impl InputState {
    pub(crate) fn observe_touch(&mut self, touch: &TouchData) {
        self.last_touch_timestamp_ms = touch.timestamp_ms;
        if touch.touch_type == TouchType::Cancel {
            self.active_touches.fill(None);
            return;
        }

        let count = usize::from(touch.count).min(touch.points.len());
        for point in touch.points[..count].iter().copied() {
            if point.flags & TOUCH_REMOVED != 0 {
                if let Some(slot) = self
                    .active_touches
                    .iter_mut()
                    .find(|slot| slot.is_some_and(|active| active.id == point.id))
                {
                    *slot = None;
                }
                continue;
            }

            let slot = self
                .active_touches
                .iter()
                .position(|slot| slot.is_some_and(|active| active.id == point.id))
                .or_else(|| self.active_touches.iter().position(Option::is_none));
            if let Some(slot) = slot {
                self.active_touches[slot] = Some(point);
            }
        }
    }

    pub(crate) fn observe_mouse_down(&mut self, x: f32, y: f32, button: u32, timestamp_ms: f64) {
        self.observe_pointer_position(x, y, timestamp_ms);
        if let Some(pressed) = self.pointer_buttons.get_mut(button as usize) {
            *pressed = true;
        }
    }

    pub(crate) fn observe_mouse_move(&mut self, x: f32, y: f32, _button: u32, timestamp_ms: f64) {
        self.observe_pointer_position(x, y, timestamp_ms);
    }

    pub(crate) fn observe_mouse_up(&mut self, x: f32, y: f32, button: u32, timestamp_ms: f64) {
        self.observe_pointer_position(x, y, timestamp_ms);
        if let Some(pressed) = self.pointer_buttons.get_mut(button as usize) {
            *pressed = false;
        }
    }

    fn observe_pointer_position(&mut self, x: f32, y: f32, timestamp_ms: f64) {
        self.pointer_position = Some(PointerPosition { x, y, timestamp_ms });
    }

    pub(crate) fn observe_key_down(
        &mut self,
        key: String,
        code: String,
        timestamp_ms: f64,
        modifiers: u32,
    ) {
        self.keys.insert(
            code,
            KeyState {
                key,
                timestamp_ms,
                modifiers,
            },
        );
    }

    pub(crate) fn observe_key_up(&mut self, code: &str) {
        self.keys.remove(code);
    }

    pub(crate) fn observe_composition_start(&mut self) {
        self.composition_active = true;
    }

    pub(crate) fn observe_composition_end(&mut self) {
        self.composition_active = false;
    }

    pub(crate) fn retract_for_focus_loss(&mut self, mut dispatch: impl FnMut(InputRetraction)) {
        let mut cancelled = TouchData {
            touch_type: TouchType::Cancel,
            count: 0,
            points: [TouchPoint::default(); 10],
            timestamp_ms: self.last_touch_timestamp_ms,
        };
        for slot in &mut self.active_touches {
            if let Some(mut point) = slot.take() {
                point.flags |= TOUCH_CHANGED | TOUCH_REMOVED;
                cancelled.points[usize::from(cancelled.count)] = point;
                cancelled.count += 1;
            }
        }
        if cancelled.count != 0 {
            dispatch(InputRetraction::TouchCancel(cancelled));
        }

        if let Some(position) = self.pointer_position {
            for (button, pressed) in self.pointer_buttons.iter_mut().enumerate() {
                if std::mem::take(pressed) {
                    dispatch(InputRetraction::MouseUp {
                        x: position.x,
                        y: position.y,
                        button: button as u32,
                        timestamp_ms: position.timestamp_ms,
                    });
                }
            }
        }

        for (code, key) in self.keys.drain() {
            dispatch(InputRetraction::KeyUp {
                key: key.key,
                code,
                timestamp_ms: key.timestamp_ms,
                modifiers: key.modifiers,
            });
        }

        if std::mem::take(&mut self.composition_active) {
            dispatch(InputRetraction::CompositionEnd);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(id: u32, x: f32, flags: u32) -> TouchPoint {
        TouchPoint {
            id,
            x,
            y: x + 1.0,
            pressure: 0.5,
            flags,
        }
    }

    fn touch(touch_type: TouchType, points: &[TouchPoint], timestamp_ms: i64) -> TouchData {
        let mut all = [TouchPoint::default(); 10];
        all[..points.len()].copy_from_slice(points);
        TouchData {
            touch_type,
            count: points.len() as u8,
            points: all,
            timestamp_ms,
        }
    }

    #[test]
    fn focus_loss_retracts_every_active_stream_once() {
        let mut state = InputState::default();
        state.observe_touch(&touch(TouchType::Start, &[point(7, 1.0, 1)], 10));
        state.observe_touch(&touch(TouchType::Move, &[point(7, 2.0, 1)], 11));
        state.observe_mouse_down(3.0, 4.0, 0, 12.0);
        state.observe_mouse_move(5.0, 6.0, 0, 13.0);
        state.observe_key_down("a".to_owned(), "KeyA".to_owned(), 14.0, 2);
        state.observe_composition_start();

        let mut got = Vec::new();
        state.retract_for_focus_loss(|event| got.push(event));

        match &got[0] {
            InputRetraction::TouchCancel(touch) => {
                assert_eq!(touch.touch_type, TouchType::Cancel);
                assert_eq!(touch.count, 1);
                assert_eq!(touch.points[0].id, 7);
                assert_eq!(touch.points[0].x, 2.0);
                assert_eq!(touch.points[0].flags & 3, 3);
                assert_eq!(touch.timestamp_ms, 11);
            }
            other => panic!("expected touch cancellation, got {other:?}"),
        }
        assert!(matches!(
            got[1],
            InputRetraction::MouseUp {
                x: 5.0,
                y: 6.0,
                button: 0,
                timestamp_ms: 13.0,
            }
        ));
        assert!(matches!(
            &got[2],
            InputRetraction::KeyUp {
                key,
                code,
                timestamp_ms: 14.0,
                modifiers: 2,
            } if key == "a" && code == "KeyA"
        ));
        assert!(matches!(got[3], InputRetraction::CompositionEnd));

        let mut second = Vec::new();
        state.retract_for_focus_loss(|event| second.push(event));
        assert!(second.is_empty());
    }

    #[test]
    fn completed_streams_are_not_retracted() {
        let mut state = InputState::default();
        state.observe_touch(&touch(
            TouchType::Start,
            &[point(1, 1.0, 1), point(2, 2.0, 1)],
            1,
        ));
        state.observe_touch(&touch(
            TouchType::End,
            &[point(1, 3.0, 0), point(2, 4.0, 3)],
            2,
        ));
        state.observe_mouse_down(1.0, 2.0, 1, 3.0);
        state.observe_mouse_up(2.0, 3.0, 1, 4.0);
        state.observe_key_down("b".to_owned(), "KeyB".to_owned(), 5.0, 0);
        state.observe_key_up("KeyB");
        state.observe_composition_start();
        state.observe_composition_end();

        let mut got = Vec::new();
        state.retract_for_focus_loss(|event| got.push(event));

        assert_eq!(got.len(), 1);
        match &got[0] {
            InputRetraction::TouchCancel(touch) => {
                assert_eq!(touch.count, 1);
                assert_eq!(touch.points[0].id, 1);
                assert_eq!(touch.points[0].x, 3.0);
            }
            other => panic!("expected remaining touch cancellation, got {other:?}"),
        }
    }

    #[test]
    fn repeated_key_down_updates_the_single_retraction() {
        let mut state = InputState::default();
        state.observe_key_down("a".to_owned(), "KeyA".to_owned(), 1.0, 0);
        state.observe_key_down("A".to_owned(), "KeyA".to_owned(), 2.0, 1);

        let mut got = Vec::new();
        state.retract_for_focus_loss(|event| got.push(event));

        assert_eq!(got.len(), 1);
        assert!(matches!(
            &got[0],
            InputRetraction::KeyUp {
                key,
                code,
                timestamp_ms: 2.0,
                modifiers: 1,
            } if key == "A" && code == "KeyA"
        ));
    }
}
