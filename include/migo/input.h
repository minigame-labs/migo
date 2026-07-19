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

MIGO_END_DECLS

#endif /* MIGO_INPUT_H */
