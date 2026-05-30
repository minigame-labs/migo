// EventTarget — minimal, matches the surface games expect.
// addEventListener / removeEventListener / dispatchEvent.
// Listeners are stored in a plain map keyed by event type.

export default class EventTarget {
  constructor() {
    this._listeners = {};
  }

  addEventListener(type, listener /* , optionsOrCapture */) {
    if (typeof listener !== "function") return;
    const list = this._listeners[type] || (this._listeners[type] = []);
    if (list.indexOf(listener) === -1) list.push(listener);
  }

  removeEventListener(type, listener) {
    const list = this._listeners[type];
    if (!list) return;
    const i = list.indexOf(listener);
    if (i !== -1) list.splice(i, 1);
  }

  dispatchEvent(event) {
    if (!event || !event.type) return false;
    const list = this._listeners[event.type];
    if (!list || list.length === 0) return true;
    if (event.target == null) event.target = this;
    if (event.currentTarget == null) event.currentTarget = this;
    // Iterate over a copy: a listener may add/remove during dispatch.
    const snapshot = list.slice();
    for (let i = 0; i < snapshot.length; i++) {
      try { snapshot[i].call(this, event); } catch (e) {
        // Match browser behavior: a listener throwing must not stop others.
        if (typeof console !== "undefined" && console.error) console.error(e);
      }
    }
    return !event.defaultPrevented;
  }
}
