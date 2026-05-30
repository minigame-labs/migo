// Audio — `new Audio(src)` browser-style; bridge to migo.createInnerAudioContext.
// Browser Audio mostly exposes: src, play(), pause(), loop, volume, currentTime,
// addEventListener('canplay'/'play'/'pause'/'ended'/'error'). InnerAudioContext
// has the same shape with on{Play,Pause,Ended,Error}. Wire them up.

import EventTarget from "./event-target.js";

export default class Audio extends EventTarget {
  constructor(src) {
    super();
    if (typeof migo.createInnerAudioContext !== "function") {
      throw new Error("[migo-adapter] migo.createInnerAudioContext is not available");
    }
    this._ctx = migo.createInnerAudioContext();
    this._readyState = 0; // 0 = HAVE_NOTHING

    const dispatchSelf = (type) => () => {
      if (type === "canplay") this._readyState = 4;
      this.dispatchEvent({ type });
    };
    this._ctx.onCanplay && this._ctx.onCanplay(dispatchSelf("canplay"));
    this._ctx.onPlay && this._ctx.onPlay(dispatchSelf("play"));
    this._ctx.onPause && this._ctx.onPause(dispatchSelf("pause"));
    this._ctx.onEnded && this._ctx.onEnded(dispatchSelf("ended"));
    this._ctx.onError && this._ctx.onError((err) => this.dispatchEvent({ type: "error", error: err }));

    if (src) this.src = src;
  }

  set src(v) { this._ctx.src = v; }
  get src() { return this._ctx.src; }

  set loop(v) { this._ctx.loop = v; }
  get loop() { return this._ctx.loop; }

  set volume(v) { this._ctx.volume = v; }
  get volume() { return this._ctx.volume; }

  set autoplay(v) { this._ctx.autoplay = v; }
  get autoplay() { return this._ctx.autoplay; }

  get currentTime() { return this._ctx.currentTime || 0; }
  set currentTime(v) { if (typeof this._ctx.seek === "function") this._ctx.seek(v); }
  get duration() { return this._ctx.duration || 0; }
  get paused() { return !!this._ctx.paused; }
  get readyState() { return this._readyState; }

  play() { this._ctx.play(); }
  pause() { this._ctx.pause(); }

  load() {} // no-op — InnerAudioContext loads on src set / play
  cloneNode() { return new Audio(this.src); }
}
