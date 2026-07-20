// W3C Gamepad API, mapped onto the runtime's `migo.*` gamepad transport.
//
// wx has no gamepad API, so the reference is the Web platform Migo replaces:
// content calls navigator.getGamepads() and listens for gamepadconnected on
// window. The runtime already implements the hard parts (null slots, frozen
// views, live state) in input/04_gamepad.js and exposes them on `migo` only.
// This module is the browser-shaped surface over that transport; it adds no
// state of its own, because a second copy of the state could disagree with the
// first.

import { Event } from "./events.js";

// https://w3c.github.io/gamepad/#gamepadevent-interface
export class GamepadEvent extends Event {
  constructor(type, init = {}) {
    super(type, init);
    this.gamepad = init.gamepad != null ? init.gamepad : null;
  }
}

// Returned when the runtime has no gamepad transport at all. Frozen and shared
// rather than freshly allocated: content polls this every frame, and a runtime
// without gamepads would otherwise allocate a garbage array per frame forever.
// Freezing is what makes sharing safe.
const _NO_GAMEPADS = Object.freeze([]);

function _transport() {
  try {
    return typeof migo !== "undefined" ? migo : null;
  } catch {
    return null;
  }
}

/*
 * navigator.getGamepads().
 *
 * A straight pass-through, deliberately. Content calls this once per frame per
 * pad, so wrapping or copying the runtime's Gamepad views here would allocate
 * on the hottest input path -- and worse, it would break object identity, which
 * content relies on to compare the pad it stored last frame against this one.
 * The runtime already returns a fresh array of stable frozen views, which is
 * exactly the Web contract.
 */
export function getGamepads() {
  const runtime = _transport();
  if (!runtime || typeof runtime.getGamepads !== "function") return _NO_GAMEPADS;
  const pads = runtime.getGamepads();
  return pads != null ? pads : _NO_GAMEPADS;
}

/*
 * Route the runtime's connect/disconnect notifications to a browser-shaped
 * dispatch, given by the caller so this module needs no opinion about what a
 * "window" is.
 *
 * Returns whether a transport was found, so the caller can tell "no gamepad
 * support" from "supported but nothing plugged in".
 */
export function connectGamepadEvents(dispatch) {
  const runtime = _transport();
  if (!runtime) return false;

  const forward = (type) => (payload) => {
    // The transport delivers its own `{ type, gamepad }` record, not the pad on
    // its own. Only the pad crosses into the Web-shaped event: it is the
    // runtime's frozen view, passed through untouched so that
    // `event.gamepad === navigator.getGamepads()[gamepad.index]` holds for a
    // connected pad -- the identity content compares against between frames.
    const gamepad = payload != null && payload.gamepad != null ? payload.gamepad : null;
    dispatch(new GamepadEvent(type, { gamepad }));
  };

  let wired = false;
  if (typeof runtime.onGamepadConnected === "function") {
    runtime.onGamepadConnected(forward("gamepadconnected"));
    wired = true;
  }
  if (typeof runtime.onGamepadDisconnected === "function") {
    runtime.onGamepadDisconnected(forward("gamepaddisconnected"));
    wired = true;
  }
  return wired;
}
