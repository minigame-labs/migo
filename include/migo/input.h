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

MIGO_END_DECLS

#endif /* MIGO_INPUT_H */
