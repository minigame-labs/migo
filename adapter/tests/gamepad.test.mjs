// W3C Gamepad API surface, tested against the REAL runtime transport.
//
// input/04_gamepad.js is dependency-free ESM, so this imports it directly
// instead of faking it. That matters: the properties under test here -- null
// slots, frozen views, object identity -- are the runtime's to provide, and a
// hand-written fake would happily agree with a broken adapter about all three.
// Importing the real module means this test fails if either side drifts.
//
// Run with `node tests/gamepad.test.mjs` from this dir or via npm test.

import assert from "node:assert/strict";

const runtime = await import(
  "../../engine/crates/js-runtime/input/04_gamepad.js"
);

// ---- Fake migo carrying the real gamepad transport ------------------------
// Only the public `migo` names are wired. The `_internalTrigger*` functions are
// driven directly from `runtime` below, standing in for the native host, so
// this test also demonstrates the adapter never needs them.
globalThis.migo = {
  getWindowInfo: () => ({ windowWidth: 390, windowHeight: 844, screenWidth: 390, screenHeight: 844, pixelRatio: 3 }),
  getSystemInfoSync: () => ({ platform: "android", system: "Android 14", language: "zh-CN", version: "1.2.3" }),
  createCanvas: () => ({ width: 0, height: 0, getContext: () => ({}), toDataURL: () => "data:," }),
  createImage: () => ({ src: null, onload: null, onerror: null, width: 0, height: 0 }),

  getGamepads: runtime.getGamepads,
  onGamepadConnected: runtime.onGamepadConnected,
  offGamepadConnected: runtime.offGamepadConnected,
  onGamepadDisconnected: runtime.onGamepadDisconnected,
  offGamepadDisconnected: runtime.offGamepadDisconnected,
};

await import("../src/index.js");

const { navigator, GamepadEvent, Event } = globalThis;

// Capture console.error so the deliberate throws below do not look like real
// failures in the output, and so isolation can be asserted rather than assumed.
const _errors = [];
const _realError = console.error;
console.error = (...args) => { _errors.push(args.join(" ")); };

// ---- 1. No pads connected yet ---------------------------------------------
assert.equal(typeof navigator.getGamepads, "function", "navigator.getGamepads exists");
assert.deepEqual([...navigator.getGamepads()], [], "no pads before any connect");

// ---- 2. Connection fires a GamepadEvent on window -------------------------
const connectEvents = [];
globalThis.addEventListener("gamepadconnected", (e) => connectEvents.push(e));

// Connect at index 2, skipping 0 and 1, to pin the sparse-slot rule below.
runtime._internalTriggerGamepadConnected(2, "Test Pad", "standard", 4, 8);

assert.equal(connectEvents.length, 1, "one gamepadconnected event");
const connectEvent = connectEvents[0];
assert.ok(connectEvent instanceof GamepadEvent, "event is a GamepadEvent");
assert.ok(connectEvent instanceof Event, "GamepadEvent extends Event");
assert.equal(connectEvent.type, "gamepadconnected");

// ---- 3. Empty slots are explicit nulls, not holes -------------------------
// Content stores an index, so a missing pad must read as null rather than
// shifting later pads down or exposing an array hole as undefined.
const pads = navigator.getGamepads();
assert.equal(pads.length, 3, "array is long enough to index the connected pad");
assert.equal(pads[0], null, "slot 0 is an explicit null");
assert.equal(pads[1], null, "slot 1 is an explicit null");
assert.ok(pads[2], "slot 2 holds the pad");
assert.ok(!(0 in pads) === false, "slot 0 is a real element, not a hole");
assert.equal(pads[2].index, 2, "pad reports the index it lives at");
assert.equal(pads[2].id, "Test Pad");
assert.equal(pads[2].mapping, "standard");

// ---- 4. Object identity ----------------------------------------------------
// Content compares the pad it stored last frame against this frame's. Wrapping
// or copying in the adapter would break that comparison silently.
assert.equal(connectEvent.gamepad, pads[2], "event pad is the same object getGamepads returns");
assert.equal(navigator.getGamepads()[2], pads[2], "pad identity is stable across calls");
assert.notEqual(navigator.getGamepads(), pads, "each call returns a fresh array, per spec");

// ---- 5. Button and axis counts are known at connection --------------------
// Content commonly reads buttons.length inside its gamepadconnected listener to
// decide a control layout, so the arrays must already be the right size there.
assert.equal(connectEvent.gamepad.axes.length, 4, "axis count known at connect");
assert.equal(connectEvent.gamepad.buttons.length, 8, "button count known at connect");

// ---- 6. The view is frozen -------------------------------------------------
const pad = pads[2];
assert.ok(Object.isFrozen(pad), "gamepad view is frozen");
assert.ok(Object.isFrozen(pad.axes), "axes array is frozen");
assert.ok(Object.isFrozen(pad.buttons), "buttons array is frozen");
assert.ok(Object.isFrozen(pad.buttons[0]), "each button is frozen");
assert.throws(() => { pad.id = "hacked"; }, TypeError, "cannot overwrite pad fields");
assert.throws(() => { pad.axes[0] = 1; }, TypeError, "cannot overwrite an axis");

// ---- 7. Live state still updates through the frozen view -------------------
// Frozen means the shape is fixed, not that the values are. `pressed` is
// carried, never derived from `value`: the device decides its own threshold.
runtime._internalTriggerGamepadState(2, 1234, [4, 8, 0.5, 0, 0, 0,
  1, 1, 1.0,   // button 0: pressed
  0, 0, 0.25,  // button 1: NOT pressed despite a non-zero value
  0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
assert.equal(pad.axes[0], 0.5, "axis value updates through the frozen view");
assert.equal(pad.timestamp, 1234, "timestamp updates");
assert.equal(pad.buttons[0].pressed, true, "digital press reported");
assert.equal(pad.buttons[1].pressed, false, "a non-zero value is not itself a press");
assert.equal(pad.buttons[1].value, 0.25, "analogue value preserved");

// ---- 8. Listener mutation during dispatch ---------------------------------
// Removing a listener from inside a listener must not skip the next one.
const order = [];
const second = () => order.push("second");
const first = () => {
  order.push("first");
  globalThis.removeEventListener("gamepadconnected", second);
};
globalThis.addEventListener("gamepadconnected", first);
globalThis.addEventListener("gamepadconnected", second);
runtime._internalTriggerGamepadConnected(0, "Second Pad", "standard", 2, 4);
assert.deepEqual(order, ["first", "second"], "removal mid-dispatch does not skip a listener");
globalThis.removeEventListener("gamepadconnected", first);

// The pad that just connected fills the previously-null slot 0.
assert.equal(navigator.getGamepads()[0].id, "Second Pad", "slot 0 now holds a pad");

// ---- 9. Exception isolation ------------------------------------------------
const survived = [];
const thrower = () => { throw new Error("listener blew up"); };
const survivor = () => survived.push("ran");
globalThis.addEventListener("gamepaddisconnected", thrower);
globalThis.addEventListener("gamepaddisconnected", survivor);

// ---- 10. Disconnect --------------------------------------------------------
const disconnectEvents = [];
globalThis.addEventListener("gamepaddisconnected", (e) => disconnectEvents.push(e));
runtime._internalTriggerGamepadDisconnected(2);

assert.deepEqual(survived, ["ran"], "a throwing listener does not stop the others");
assert.ok(_errors.length > 0, "the failure was reported rather than swallowed");
assert.equal(disconnectEvents.length, 1, "one gamepaddisconnected event");
assert.equal(disconnectEvents[0].type, "gamepaddisconnected");
assert.equal(disconnectEvents[0].gamepad, pad, "disconnect carries the pad that left");
assert.equal(pad.connected, false, "the departed pad reports disconnected");
assert.equal(navigator.getGamepads()[2], null, "its slot is emptied, not removed");
assert.equal(navigator.getGamepads().length, 3, "later indices are not shifted down");

// ---- 11. The runtime transport stays private -------------------------------
// The adapter maps `migo.*` to Web names; it must not republish the native
// pump hooks, which content has no business calling.
for (const hidden of [
  "_internalTriggerGamepadConnected",
  "_internalTriggerGamepadDisconnected",
  "_internalTriggerGamepadState",
  "onGamepadConnected",
  "offGamepadConnected",
]) {
  assert.equal(globalThis[hidden], undefined, `${hidden} is not on globalThis`);
  assert.equal(navigator[hidden], undefined, `${hidden} is not on navigator`);
}

console.error = _realError;
console.log("ALL GAMEPAD ASSERTIONS PASSED (11 sections)");
