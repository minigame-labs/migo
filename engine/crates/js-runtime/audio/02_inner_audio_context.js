
import {
  op_inner_audio_create,
  op_inner_audio_destroy,
  op_inner_audio_load_url,
  op_inner_audio_play,
  op_inner_audio_pause,
  op_inner_audio_stop,
  op_inner_audio_seek,
  op_inner_audio_set_volume,
  op_inner_audio_set_loop,
  op_inner_audio_set_playback_rate,
  op_inner_audio_set_autoplay,
} from "ext:core/ops";

// ID counter for InnerAudioContext instances
let nextInnerAudioId = 1;

// Registry of all active InnerAudioContext instances (for event dispatch)
const audioContextRegistry = new Map();

// Event queue and scheduling (microtask batching like touch events)
let _eventQueue = [];
let _scheduled = false;

/**
 * Called from native (Rust / V8 binding) to enqueue audio events.
 * Events are batched and dispatched in the next microtask.
 *
 * @param {number} id - InnerAudioContext ID
 * @param {string} eventType - Event type (canPlay, play, pause, etc.)
 * @param {number} currentTime - Current playback time
 */
function _internalEnqueueInnerAudioEvent(id, eventType, currentTime) {
  _eventQueue.push({ id, eventType, currentTime });

  if (!_scheduled) {
    _scheduled = true;
    Promise.resolve().then(_drainEvents);
  }
}

/**
 * Drain and dispatch all queued events
 */
function _drainEvents() {
  _scheduled = false;

  // Swap queue to avoid re-entrancy issues
  const batch = _eventQueue;
  _eventQueue = [];

  for (const event of batch) {
    const ctx = audioContextRegistry.get(event.id);
    if (ctx) {
      ctx._handleNativeEvent(event);
    }
  }
}

/**
 * Register a context for event dispatch
 */
function registerContext(ctx) {
  audioContextRegistry.set(ctx._getId(), ctx);
}

/**
 * Unregister a context
 */
function unregisterContext(ctx) {
  audioContextRegistry.delete(ctx._getId());
}

class InnerAudioContext {
  #id;
  #src = "";
  #startTime = 0;
  #autoplay = false;
  #loop = false;
  #obeyMuteSwitch = true;
  #volume = 1.0;
  #playbackRate = 1.0;

  // Read-only properties (cached from native)
  #duration = 0;
  #currentTime = 0;
  #paused = true;
  #buffered = false;

  // Event callbacks
  #onCanplay = null;
  #onPlay = null;
  #onPause = null;
  #onStop = null;
  #onEnded = null;
  #onTimeUpdate = null;
  #onError = null;
  #onWaiting = null;
  #onSeeking = null;
  #onSeeked = null;

  // Loading state
  #destroyed = false;

  constructor() {
    this.#id = nextInnerAudioId++;
    op_inner_audio_create(this.#id);
    registerContext(this);
  }

  /** Internal: Get ID for event registry */
  _getId() {
    return this.#id;
  }

  /** Internal: Handle native event */
  _handleNativeEvent(event) {
    if (this.#destroyed) return;

    // Update current time from event
    this.#currentTime = event.currentTime;

    switch (event.eventType) {
      case "canPlay":
        this.#buffered = true;
        this.#fireCallback(this.#onCanplay);
        break;
      case "play":
        this.#paused = false;
        this.#fireCallback(this.#onPlay);
        break;
      case "pause":
        this.#paused = true;
        this.#fireCallback(this.#onPause);
        break;
      case "stop":
        this.#paused = true;
        this.#currentTime = 0;
        this.#fireCallback(this.#onStop);
        break;
      case "ended":
        this.#paused = true;
        this.#currentTime = 0;
        this.#fireCallback(this.#onEnded);
        break;
      case "seeking":
        this.#fireCallback(this.#onSeeking);
        break;
      case "seeked":
        this.#fireCallback(this.#onSeeked);
        break;
      case "timeUpdate":
        this.#fireCallback(this.#onTimeUpdate);
        break;
      case "error":
        this.#fireCallback(this.#onError, { errMsg: "Playback error" });
        break;
    }
  }

  #fireCallback(fn, arg) {
    if (fn) {
      try {
        fn(arg);
      } catch (e) {
        console.error("InnerAudioContext callback error:", e);
      }
    }
  }

  // ==================== Properties ====================

  /** Audio source URL or path */
  get src() {
    return this.#src;
  }

  set src(value) {
    if (this.#src === value) return;
    this.#src = value;
    this.#loadAudio();
  }

  /** Start position in seconds */
  get startTime() {
    return this.#startTime;
  }

  set startTime(value) {
    this.#startTime = Math.max(0, value);
  }

  /** Whether to autoplay */
  get autoplay() {
    return this.#autoplay;
  }

  set autoplay(value) {
    this.#autoplay = !!value;
    op_inner_audio_set_autoplay(this.#id, this.#autoplay);
  }

  /** Whether to loop */
  get loop() {
    return this.#loop;
  }

  set loop(value) {
    this.#loop = !!value;
    op_inner_audio_set_loop(this.#id, this.#loop);
  }

  /** Whether to obey mute switch (iOS only, always true on Android) */
  get obeyMuteSwitch() {
    return this.#obeyMuteSwitch;
  }

  set obeyMuteSwitch(value) {
    this.#obeyMuteSwitch = !!value;
  }

  /** Volume (0.0 - 1.0) */
  get volume() {
    return this.#volume;
  }

  set volume(value) {
    this.#volume = Math.max(0, Math.min(1, value));
    op_inner_audio_set_volume(this.#id, this.#volume);
  }

  /** Playback rate (0.5 - 2.0) */
  get playbackRate() {
    return this.#playbackRate;
  }

  set playbackRate(value) {
    this.#playbackRate = Math.max(0.5, Math.min(2.0, value));
    op_inner_audio_set_playback_rate(this.#id, this.#playbackRate);
  }

  /** Duration in seconds (read-only) */
  get duration() {
    return this.#duration;
  }

  /** Current playback position in seconds */
  get currentTime() {
    return this.#currentTime;
  }

  /** Set current playback position (equivalent to calling seek) */
  set currentTime(value) {
    this.seek(value);
  }

  /** Whether audio is paused (read-only) */
  get paused() {
    return this.#paused;
  }

  /** Buffered time percentage 0-100 (read-only) */
  get buffered() {
    return this.#buffered ? 100 : 0;
  }

  // ==================== Methods ====================

  /** Start playback */
  play() {
    if (this.#destroyed) return;

    if (!this.#buffered) {
      // Not loaded yet, set autoplay
      this.#autoplay = true;
      op_inner_audio_set_autoplay(this.#id, true);
      return;
    }

    // Seek to startTime if specified and at the beginning
    if (this.#startTime > 0 && this.#currentTime === 0) {
      op_inner_audio_seek(this.#id, this.#startTime);
    }

    op_inner_audio_play(this.#id);
    // Note: Don't update #paused here - wait for native event
  }

  /** Pause playback */
  pause() {
    if (this.#destroyed) return;
    op_inner_audio_pause(this.#id);
    // Note: Don't update #paused here - wait for native event
  }

  /** Stop playback and reset position */
  stop() {
    if (this.#destroyed) return;
    op_inner_audio_stop(this.#id);
    // Note: Don't update state here - wait for native event
  }

  /** Seek to position in seconds */
  seek(position) {
    if (this.#destroyed) return;
    op_inner_audio_seek(this.#id, position);
    // Note: Events will be fired from native
  }

  /** Destroy the audio context */
  destroy() {
    if (this.#destroyed) return;
    this.#destroyed = true;
    unregisterContext(this);
    op_inner_audio_destroy(this.#id);
  }

  /** Register callback when audio can play */
  onCanplay(fn) {
    this.#onCanplay = typeof fn === "function" ? fn : null;
  }

  /** Register callback when playback starts */
  onPlay(fn) {
    this.#onPlay = typeof fn === "function" ? fn : null;
  }

  /** Register callback when playback pauses */
  onPause(fn) {
    this.#onPause = typeof fn === "function" ? fn : null;
  }

  /** Register callback when playback stops */
  onStop(fn) {
    this.#onStop = typeof fn === "function" ? fn : null;
  }

  /** Register callback when playback ends */
  onEnded(fn) {
    this.#onEnded = typeof fn === "function" ? fn : null;
  }

  /** Register callback periodically during playback */
  onTimeUpdate(fn) {
    this.#onTimeUpdate = typeof fn === "function" ? fn : null;
  }

  /** Register callback on error */
  onError(fn) {
    this.#onError = typeof fn === "function" ? fn : null;
  }

  /** Register callback when waiting for data */
  onWaiting(fn) {
    this.#onWaiting = typeof fn === "function" ? fn : null;
  }

  /** Register callback when seeking */
  onSeeking(fn) {
    this.#onSeeking = typeof fn === "function" ? fn : null;
  }

  /** Register callback when seek completes */
  onSeeked(fn) {
    this.#onSeeked = typeof fn === "function" ? fn : null;
  }

  offCanplay(fn) {
    if (!fn || this.#onCanplay === fn) this.#onCanplay = null;
  }

  offPlay(fn) {
    if (!fn || this.#onPlay === fn) this.#onPlay = null;
  }

  offPause(fn) {
    if (!fn || this.#onPause === fn) this.#onPause = null;
  }

  offStop(fn) {
    if (!fn || this.#onStop === fn) this.#onStop = null;
  }

  offEnded(fn) {
    if (!fn || this.#onEnded === fn) this.#onEnded = null;
  }

  offTimeUpdate(fn) {
    if (!fn || this.#onTimeUpdate === fn) this.#onTimeUpdate = null;
  }

  offError(fn) {
    if (!fn || this.#onError === fn) this.#onError = null;
  }

  offWaiting(fn) {
    if (!fn || this.#onWaiting === fn) this.#onWaiting = null;
  }

  offSeeking(fn) {
    if (!fn || this.#onSeeking === fn) this.#onSeeking = null;
  }

  offSeeked(fn) {
    if (!fn || this.#onSeeked === fn) this.#onSeeked = null;
  }

  // ==================== Internal Methods ====================

  async #loadAudio() {
    if (!this.#src || this.#destroyed) return;

    this.#buffered = false;

    try {
      // Use streaming load - Rust handles HTTP download and progressive decoding
      // This enables edge-download-edge-play for faster playback start
      await op_inner_audio_load_url(this.#id, this.#src);

      // Note: canplay event will be fired from native via push once enough data is buffered
      // Duration will be updated as streaming progresses
      // Autoplay is also handled by native
    } catch (e) {
      if (this.#onError) {
        try {
          this.#onError({ errMsg: e.message || String(e) });
        } catch (e2) {
          console.error("InnerAudioContext onError error:", e2);
        }
      }
    }
  }
}

function createInnerAudioContext() {
  return new InnerAudioContext();
}

export { InnerAudioContext, createInnerAudioContext, _internalEnqueueInnerAudioEvent };
