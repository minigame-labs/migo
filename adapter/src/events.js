// Browser-style Event classes the engines occasionally `instanceof` against.
// All extend a plain Event base; the adapter never actually constructs them
// internally — touch/mouse events are dispatched as plain objects with a
// matching `type` field.

export class Event {
  constructor(type, init = {}) {
    this.type = type;
    this.bubbles = !!init.bubbles;
    this.cancelable = !!init.cancelable;
    this.target = null;
    this.currentTarget = null;
    this.timeStamp = Date.now();
    this.defaultPrevented = false;
  }
  preventDefault() { if (this.cancelable) this.defaultPrevented = true; }
  stopPropagation() {}
  stopImmediatePropagation() {}
}

export class TouchEvent extends Event {
  constructor(type, init = {}) {
    super(type, init);
    this.touches = init.touches || [];
    this.targetTouches = init.targetTouches || this.touches;
    this.changedTouches = init.changedTouches || this.touches;
  }
}

export class MouseEvent extends Event {
  constructor(type, init = {}) {
    super(type, init);
    this.clientX = init.clientX || 0;
    this.clientY = init.clientY || 0;
    this.pageX = init.pageX || this.clientX;
    this.pageY = init.pageY || this.clientY;
    this.button = init.button || 0;
    this.buttons = init.buttons || 0;
  }
}

export class DeviceMotionEvent extends Event {
  constructor(type, init = {}) {
    super(type, init);
    this.acceleration = init.acceleration || null;
    this.accelerationIncludingGravity = init.accelerationIncludingGravity || null;
    this.rotationRate = init.rotationRate || null;
    this.interval = init.interval || 0;
  }
}
