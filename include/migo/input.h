/*
 * Migo C ABI: input.
 *
 * Pointer input only in this slice. Keyboard, text/IME and gamepad contracts are
 * still open freeze blockers (see README.md).
 */
#ifndef MIGO_INPUT_H
#define MIGO_INPUT_H

#include <stddef.h> /* offsetof */
#include <migo/types.h>
#include <migo/session.h>

MIGO_BEGIN_DECLS

/*
 * Touch, including on desktop. wx mini-game content listens for
 * touchstart/touchmove/touchend and nothing else, so a host with a mouse maps it
 * to a single touch point with id 0 rather than expecting a separate pointer
 * event model.
 */
typedef uint32_t MigoTouchType;
#define MIGO_TOUCH_START UINT32_C(0)
#define MIGO_TOUCH_MOVE UINT32_C(1)
#define MIGO_TOUCH_END UINT32_C(2)
#define MIGO_TOUCH_CANCEL UINT32_C(3)

typedef uint32_t MigoTouchPointFlags;
#define MIGO_TOUCH_FLAG_NONE UINT32_C(0)
/* This point is part of the event's changedTouches list. */
#define MIGO_TOUCH_FLAG_CHANGED (UINT32_C(1) << 0)
/* This point left the surface with this event, so it is excluded from touches. */
#define MIGO_TOUCH_FLAG_REMOVED (UINT32_C(1) << 1)

/* Bounded by the engine's fixed inline array; larger counts are rejected. */
#define MIGO_TOUCH_MAX_POINTS 10

/*
 * One pointer within an event.
 *
 * x and y are CSS pixels -- logical coordinates, not physical ones. A host
 * converts from its own pixels using the scale_factor it supplied at attach.
 * Sending physical pixels is the single most likely integration mistake: the
 * game renders correctly and input lands somewhere else.
 *
 * No struct_size: this is an array element, and a per-point size prefix would
 * break the single-copy delivery path. The layout is pinned by static assertion
 * on both sides of the boundary instead.
 */
typedef struct MigoTouchPoint {
    uint32_t id; /* stable across start -> move -> end for one pointer */
    float x;
    float y;
    float pressure; /* 0.0 when the device does not report pressure */
    MigoTouchPointFlags flags;
} MigoTouchPoint;

MIGO_STATIC_ASSERT(sizeof(MigoTouchPoint) == 20, "MigoTouchPoint is 20 bytes on every target");
MIGO_STATIC_ASSERT(offsetof(MigoTouchPoint, id) == 0, "MigoTouchPoint.id moved");
MIGO_STATIC_ASSERT(offsetof(MigoTouchPoint, x) == 4, "MigoTouchPoint.x moved");
MIGO_STATIC_ASSERT(offsetof(MigoTouchPoint, y) == 8, "MigoTouchPoint.y moved");
MIGO_STATIC_ASSERT(offsetof(MigoTouchPoint, pressure) == 12, "MigoTouchPoint.pressure moved");
MIGO_STATIC_ASSERT(offsetof(MigoTouchPoint, flags) == 16, "MigoTouchPoint.flags moved");

/*
 * points is borrowed for the duration of the call: the implementation copies
 * what it needs before returning, so the caller may reuse or free the array
 * immediately afterwards.
 */
typedef struct MigoTouchEvent {
    uint32_t struct_size;
    uint32_t abi_version;
    MigoTouchType type;
    uint32_t point_count; /* 1..MIGO_TOUCH_MAX_POINTS */
    int64_t timestamp_ms;
    const MigoTouchPoint *points;
} MigoTouchEvent;

MIGO_STATIC_ASSERT(offsetof(MigoTouchEvent, struct_size) == 0,
                   "every versioned struct must begin with struct_size");
#if MIGO_LP64
MIGO_STATIC_ASSERT(sizeof(MigoTouchEvent) == 32, "MigoTouchEvent LP64 size changed");
MIGO_STATIC_ASSERT(offsetof(MigoTouchEvent, timestamp_ms) == 16, "MigoTouchEvent.timestamp_ms moved");
MIGO_STATIC_ASSERT(offsetof(MigoTouchEvent, points) == 24, "MigoTouchEvent.points moved");
#endif

/*
 * Deliver one touch event to the session's content.
 *
 * Safe to call from any thread, but events are delivered in call order, so a
 * host calling concurrently from two threads gets undefined ordering. Every
 * windowing system already serialises its event loop; keep the input stream on
 * one thread.
 *
 * Returns MIGO_ERROR_INVALID_STATE when no surface is attached -- there is
 * nothing to deliver to -- and MIGO_ERROR_WOULD_BLOCK when the queue is full.
 */
MIGO_API MigoResult MIGO_CALL migo_session_send_touch(
    MigoSession *session,
    const MigoTouchEvent *event);

/*
 * Soft keyboard, wx model.
 *
 * The value a text event carries is the field's WHOLE CURRENT TEXT, not the
 * keystroke that changed it. A host that sends only the newly typed character
 * leaves content whose text never grows past one character. In the other
 * direction, MigoOnUpdateKeyboardFn is content correcting that same value.
 */
typedef uint32_t MigoKeyboardEventType;
#define MIGO_KEYBOARD_EVENT_INPUT UINT32_C(0)
#define MIGO_KEYBOARD_EVENT_CONFIRM UINT32_C(1)
#define MIGO_KEYBOARD_EVENT_COMPLETE UINT32_C(2)
#define MIGO_KEYBOARD_EVENT_HEIGHT_CHANGE UINT32_C(3)

/*
 * value_utf8 is length-delimited and need not be NUL-terminated; only
 * value_length bytes are read, and it is borrowed for the duration of the call.
 * A zero length with a non-null pointer is valid: it is the field being
 * cleared.
 *
 * height_css_px applies to MIGO_KEYBOARD_EVENT_HEIGHT_CHANGE only and is CSS
 * pixels -- logical, not physical, the same units as touch coordinates. Zero
 * means the keyboard is gone. Sending physical pixels lays content out for a
 * keyboard of the wrong size on every display whose scale factor is not 1.
 */
typedef struct MigoKeyboardEvent {
    uint32_t struct_size;
    uint32_t abi_version;
    MigoKeyboardEventType event_type;
    uint32_t value_length;
    const char *value_utf8;
    double height_css_px;
} MigoKeyboardEvent;

MIGO_STATIC_ASSERT(offsetof(MigoKeyboardEvent, struct_size) == 0,
                   "every versioned struct must begin with struct_size");
#if MIGO_LP64
MIGO_STATIC_ASSERT(sizeof(MigoKeyboardEvent) == 32, "MigoKeyboardEvent LP64 size changed");
MIGO_STATIC_ASSERT(offsetof(MigoKeyboardEvent, value_utf8) == 16,
                   "MigoKeyboardEvent.value_utf8 moved");
MIGO_STATIC_ASSERT(offsetof(MigoKeyboardEvent, height_css_px) == 24,
                   "MigoKeyboardEvent.height_css_px moved");
#endif

/*
 * Deliver one soft-keyboard event. Callable from any thread; ordering between
 * concurrent calls is the host's to guarantee.
 *
 * Returns MIGO_ERROR_INVALID_STATE when no surface is attached and
 * MIGO_ERROR_WOULD_BLOCK when the queue is full. A full queue is reported
 * rather than swallowed: a dropped COMPLETE leaves content believing the
 * keyboard is still open, and no later event corrects it.
 */
MIGO_API MigoResult MIGO_CALL migo_session_send_keyboard_event(
    MigoSession *session,
    const MigoKeyboardEvent *event);

/*
 * Physical keys.
 *
 * A different capability from the soft keyboard above, despite the shared word.
 * The soft keyboard is text: the host owns an IME and reports the field's whole
 * current value. This is a discrete press of an identified key, and content
 * reads the two through different listeners.
 *
 * Not batched, unlike touch: keys arrive at human typing speed, one at a time,
 * so a batch API would be shape without a requirement.
 */
typedef uint32_t MigoKeyEventType;
#define MIGO_KEY_EVENT_DOWN UINT32_C(0)
#define MIGO_KEY_EVENT_UP UINT32_C(1)

/*
 * key and code are DOM values, and they are NOT interchangeable:
 *
 *   code  identifies the physical key   -- "KeyA", "ArrowLeft", "Escape"
 *   key   is what it produces, given the current modifiers and layout
 *                                       -- "a", "A", "ArrowLeft"
 *
 * A host has platform keycodes -- AKEYCODE_A, an X11 keysym -- and translating
 * them is the host's work. That table lives here rather than in Migo because a
 * portable runtime that accepted platform codes would have to carry a mapping
 * per platform. Sending code in both fields is the likely mistake, and it
 * produces content that reads "KeyA" as typed text.
 *
 * An empty key is legitimate: a dead key produces no text. An empty code is
 * rejected, because a code always identifies something.
 *
 * Both strings are length-delimited, need not be NUL-terminated, and are
 * borrowed for the duration of the call.
 */
typedef struct MigoKeyEvent {
    uint32_t struct_size;
    uint32_t abi_version;
    MigoKeyEventType event_type;
    uint32_t key_length;
    const char *key_utf8;
    const char *code_utf8;
    uint32_t code_length;
    uint32_t reserved0;
    double timestamp_ms;
} MigoKeyEvent;

MIGO_STATIC_ASSERT(offsetof(MigoKeyEvent, struct_size) == 0,
                   "every versioned struct must begin with struct_size");
#if MIGO_LP64
MIGO_STATIC_ASSERT(sizeof(MigoKeyEvent) == 48, "MigoKeyEvent LP64 size changed");
MIGO_STATIC_ASSERT(offsetof(MigoKeyEvent, key_utf8) == 16, "MigoKeyEvent.key_utf8 moved");
MIGO_STATIC_ASSERT(offsetof(MigoKeyEvent, code_utf8) == 24, "MigoKeyEvent.code_utf8 moved");
MIGO_STATIC_ASSERT(offsetof(MigoKeyEvent, timestamp_ms) == 40,
                   "MigoKeyEvent.timestamp_ms moved");
#endif

/*
 * Deliver one key press or release. Callable from any thread; ordering between
 * concurrent calls is the host's to guarantee.
 *
 * A full queue is reported as MIGO_ERROR_WOULD_BLOCK rather than swallowed: a
 * dropped UP leaves content believing the key is still held, and no later event
 * corrects it.
 */
MIGO_API MigoResult MIGO_CALL migo_session_send_key_event(
    MigoSession *session,
    const MigoKeyEvent *event);

/*
 * IME composition.
 *
 * wx has no composition API, so the reference is the DOM CompositionEvent.
 * Composition is the IN-PROGRESS state of IME input: typing pinyin shows a
 * preedit string before any of it is committed. It is distinct from the soft
 * keyboard above, which reports text that has ALREADY been committed.
 *
 * A game drawing its own text field needs both -- the preedit to show what is
 * being typed, and the committed value to store -- so a host that supports an
 * IME sends composition events alongside the keyboard ones, not instead of them.
 */
typedef uint32_t MigoCompositionEventType;
#define MIGO_COMPOSITION_EVENT_START UINT32_C(0)
#define MIGO_COMPOSITION_EVENT_UPDATE UINT32_C(1)
#define MIGO_COMPOSITION_EVENT_END UINT32_C(2)

/*
 * data is the WHOLE current preedit string on start and update, never a delta:
 * content that received only what changed could not reconstruct the rest. On
 * end it is the committed text, and empty means the user cancelled -- which
 * content must still see, so it can clear the preedit it has been drawing.
 *
 * Length-delimited, need not be NUL-terminated, borrowed for the call. A length
 * that splits a multi-byte character is rejected rather than delivered mangled.
 */
typedef struct MigoCompositionEvent {
    uint32_t struct_size;
    uint32_t abi_version;
    MigoCompositionEventType event_type;
    uint32_t data_length;
    const char *data_utf8;
} MigoCompositionEvent;

MIGO_STATIC_ASSERT(offsetof(MigoCompositionEvent, struct_size) == 0,
                   "every versioned struct must begin with struct_size");
#if MIGO_LP64
MIGO_STATIC_ASSERT(sizeof(MigoCompositionEvent) == 24, "MigoCompositionEvent LP64 size changed");
MIGO_STATIC_ASSERT(offsetof(MigoCompositionEvent, data_utf8) == 16,
                   "MigoCompositionEvent.data_utf8 moved");
#endif

/*
 * Deliver one composition event.
 *
 * A full queue is MIGO_ERROR_WOULD_BLOCK rather than a silent drop: a dropped
 * END leaves content drawing a preedit that will never be cleared.
 */
MIGO_API MigoResult MIGO_CALL migo_session_send_composition_event(
    MigoSession *session,
    const MigoCompositionEvent *event);

/*
 * Gamepads.
 *
 * wx has no gamepad API, so the shape here is the Web one Migo replaces:
 * content calls navigator.getGamepads() and listens for gamepadconnected.
 * Inventing a Migo-shaped API instead would make existing HTML5 games not work
 * for no reason.
 *
 * The Web API is POLLED, not evented: content reads whatever is current each
 * frame. So a host announces a pad once, then pushes samples as often as it
 * samples, and a dropped sample is harmless because the next one replaces it
 * wholesale.
 */

/* Bounded by the engine's fixed inline arrays; larger counts are rejected
 * rather than truncated, because a dropped button is one content is watching. */
#define MIGO_GAMEPAD_MAX_AXES 8
#define MIGO_GAMEPAD_MAX_BUTTONS 20

typedef uint32_t MigoGamepadButtonFlags;
#define MIGO_GAMEPAD_BUTTON_FLAG_NONE UINT32_C(0)
#define MIGO_GAMEPAD_BUTTON_FLAG_PRESSED (UINT32_C(1) << 0)
#define MIGO_GAMEPAD_BUTTON_FLAG_TOUCHED (UINT32_C(1) << 1)

/*
 * One button. value is the analogue position -- 0.0 to 1.0 for a trigger, and
 * 0.0 or 1.0 for a digital button.
 *
 * PRESSED is carried rather than derived from value, and the two are genuinely
 * independent: a device picks its own press threshold, and a trigger resting at
 * 0.25 may or may not count as pressed depending on the pad. Deriving it here
 * would overrule the device.
 *
 * No struct_size: this is an array element, like MigoTouchPoint.
 */
typedef struct MigoGamepadButton {
    MigoGamepadButtonFlags flags;
    float value;
} MigoGamepadButton;

MIGO_STATIC_ASSERT(sizeof(MigoGamepadButton) == 8,
                   "MigoGamepadButton is 8 bytes on every target");
MIGO_STATIC_ASSERT(offsetof(MigoGamepadButton, value) == 4, "MigoGamepadButton.value moved");

/*
 * A gamepad's identity, supplied when it appears and again when it goes away.
 *
 * axis_count and button_count are given on connect rather than inferred from
 * the first sample, because getGamepads() must return correctly sized arrays
 * from the moment the gamepadconnected listener runs -- content commonly reads
 * buttons.length there to decide which layout it is looking at.
 *
 * id is the device's own name, which content shows to a player. mapping is
 * "standard" when the host has mapped the pad onto the standard layout and ""
 * when it has not; content reads it to decide whether the button order can be
 * trusted. Both are NUL-terminated and borrowed for the call.
 */
typedef struct MigoGamepadInfo {
    uint32_t struct_size;
    uint32_t abi_version;
    uint32_t index; /* the slot the pad holds for as long as it stays connected */
    uint32_t axis_count;
    uint32_t button_count;
    uint32_t reserved0;
    const char *id_utf8;
    const char *mapping_utf8;
} MigoGamepadInfo;

MIGO_STATIC_ASSERT(offsetof(MigoGamepadInfo, struct_size) == 0,
                   "every versioned struct must begin with struct_size");
#if MIGO_LP64
MIGO_STATIC_ASSERT(sizeof(MigoGamepadInfo) == 40, "MigoGamepadInfo LP64 size changed");
MIGO_STATIC_ASSERT(offsetof(MigoGamepadInfo, id_utf8) == 24, "MigoGamepadInfo.id_utf8 moved");
#endif

/*
 * One sample. axes and buttons are borrowed for the duration of the call and
 * must each hold at least their announced count.
 */
typedef struct MigoGamepadStateEvent {
    uint32_t struct_size;
    uint32_t abi_version;
    uint32_t index;
    uint32_t axis_count;
    uint32_t button_count;
    uint32_t reserved0;
    const float *axes; /* each -1.0..1.0, in the standard mapping order */
    const MigoGamepadButton *buttons;
    double timestamp_ms;
} MigoGamepadStateEvent;

MIGO_STATIC_ASSERT(offsetof(MigoGamepadStateEvent, struct_size) == 0,
                   "every versioned struct must begin with struct_size");
#if MIGO_LP64
MIGO_STATIC_ASSERT(sizeof(MigoGamepadStateEvent) == 48,
                   "MigoGamepadStateEvent LP64 size changed");
MIGO_STATIC_ASSERT(offsetof(MigoGamepadStateEvent, axes) == 24,
                   "MigoGamepadStateEvent.axes moved");
MIGO_STATIC_ASSERT(offsetof(MigoGamepadStateEvent, buttons) == 32,
                   "MigoGamepadStateEvent.buttons moved");
#endif

/*
 * Announce a gamepad, or withdraw it by passing 0 for connected. Withdrawing
 * reads only info->index, so a host may pass the same struct it connected with.
 *
 * An emptied slot stays empty rather than shifting the pads after it: content
 * holds on to an index.
 */
MIGO_API MigoResult MIGO_CALL migo_session_set_gamepad_connected(
    MigoSession *session,
    const MigoGamepadInfo *info,
    uint32_t connected);

/* Push one sample for a connected pad. */
MIGO_API MigoResult MIGO_CALL migo_session_send_gamepad_state(
    MigoSession *session,
    const MigoGamepadStateEvent *event);

MIGO_END_DECLS

#endif /* MIGO_INPUT_H */
