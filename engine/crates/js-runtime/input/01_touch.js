const TOUCH_STRIDE = 20; // bytes
const FLAG_CHANGED = 1;

// typeCode from native
const TYPE_MAP = Object.freeze({
  0: 'start',
  1: 'move',
  2: 'end',
  3: 'cancel',
});

const TOUCH_LISTENERS = {
  start: new Set(),
  move: new Set(),
  end: new Set(),
  cancel: new Set(),
};

function addListener(type, fn) {
  if (typeof fn !== 'function') return;
  const set = TOUCH_LISTENERS[type];
  if (set) set.add(fn);
}

function removeListener(type, fn) {
  const set = TOUCH_LISTENERS[type];
  if (set) set.delete(fn);
}

export const onTouchStart = (fn) => addListener('start', fn);
export const onTouchMove = (fn) => addListener('move', fn);
export const onTouchEnd = (fn) => addListener('end', fn);
export const onTouchCancel = (fn) => addListener('cancel', fn);

export const offTouchStart = (fn) => removeListener('start', fn);
export const offTouchMove = (fn) => removeListener('move', fn);
export const offTouchEnd = (fn) => removeListener('end', fn);
export const offTouchCancel = (fn) => removeListener('cancel', fn);

let _queue = [];
let _scheduled = false;

/**
 * Called from native (Rust / V8 binding)
 *
 * @param {number} typeCode
 * @param {ArrayBuffer} buffer - TouchPoint[]
 * @param {number} count
 * @param {number} timeStamp
 */
export function _internalEnqueueRawTouchEvent(typeCode, buffer, count, timeStamp) {
  _queue.push({ typeCode, buffer, count, timeStamp });

  if (!_scheduled) {
    _scheduled = true;
    Promise.resolve().then(_drain);
  }
}

function _makeTouch(id, x, y, pressure) {
  // Keep shape stable for JS engines
  return {
    identifier: id,
    clientX: x,
    clientY: y,
    pageX: x,
    pageY: y,
    force: pressure,
  };
}

function _drain() {
  _scheduled = false;

  // Swap queue to avoid re-entrancy issues
  const batch = _queue;
  _queue = [];

  for (let idx = 0; idx < batch.length; idx++) {
    const ev = batch[idx];

    const type = TYPE_MAP[ev.typeCode] || 'move';
    const listeners = TOUCH_LISTENERS[type];
    if (!listeners || listeners.size === 0) continue;

    const count = ev.count | 0;
    if (count <= 0 || !(ev.buffer instanceof ArrayBuffer)) {
      const event = {
        type,
        touches: [],
        changedTouches: [],
        timeStamp: ev.timeStamp,
      };

      for (const fn of listeners) {
        try {
          fn(event);
        } catch (_) {
          // swallow listener errors to avoid breaking input pipeline
        }
      }
      continue;
    }

    const view = new DataView(ev.buffer);
    const touches = [];
    const changedTouches = [];

    for (let i = 0; i < count; i++) {
      const base = i * TOUCH_STRIDE;

      const id = view.getUint32(base, true);
      const x = view.getFloat32(base + 4, true);
      const y = view.getFloat32(base + 8, true);
      const pressure = view.getFloat32(base + 12, true);
      const flags = view.getUint32(base + 16, true);

      const touch = _makeTouch(id, x, y, pressure);
      touches.push(touch);
      if (flags & FLAG_CHANGED) changedTouches.push(touch);
    }

    const event = {
      type,
      touches,
      changedTouches,
      timeStamp: ev.timeStamp,
    };

    for (const fn of listeners) {
      try {
        fn(event);
      } catch (_) {
        // swallow listener errors to avoid breaking input pipeline
      }
    }
  }
}
