import { op_audio_set_buffer, op_audio_start, op_audio_stop, op_audio_set_loop } from "ext:core/ops";
import { AudioParam } from "ext:host_v8_audio/00_audio_param.js";
import { AudioBuffer } from "ext:host_v8_audio/00_audio_buffer.js";
import { AudioNode } from "ext:host_v8_audio/00_audio_node.js";

class AudioBufferSourceNode extends AudioNode {
  #buffer = null;
  #loop = false;
  #loopStart = 0;
  #loopEnd = 0;
  #playbackRate;
  #detune;
  #started = false;
  #onended = null;

  constructor(context, nodeId) {
    super(context, nodeId, {
      numberOfInputs: 0,
      numberOfOutputs: 1,
    });
    this.#playbackRate = new AudioParam(1.0, -3.4028235e38, 3.4028235e38);
    this.#playbackRate._bind(nodeId, "playbackRate");
    this.#detune = new AudioParam(0, -3.4028235e38, 3.4028235e38);
    this.#detune._bind(nodeId, "detune");
  }

  get buffer() {
    return this.#buffer;
  }

  set buffer(value) {
    if (value !== null && !(value instanceof AudioBuffer)) {
      throw new TypeError("buffer must be an AudioBuffer or null");
    }
    this.#buffer = value;
    if (value) {
      op_audio_set_buffer(this._nodeId, value._id);
    }
  }

  get loop() {
    return this.#loop;
  }

  set loop(value) {
    this.#loop = Boolean(value);
    op_audio_set_loop(this._nodeId, this.#loop, this.#loopStart, this.#loopEnd);
  }

  get loopStart() {
    return this.#loopStart;
  }

  set loopStart(value) {
    this.#loopStart = Number(value) || 0;
  }

  get loopEnd() {
    return this.#loopEnd;
  }

  set loopEnd(value) {
    this.#loopEnd = Number(value) || 0;
  }

  get playbackRate() {
    return this.#playbackRate;
  }

  get detune() {
    return this.#detune;
  }

  get onended() {
    return this.#onended;
  }

  set onended(value) {
    this.#onended = typeof value === "function" ? value : null;
  }

  start(when = 0, offset = 0, duration) {
    if (this.#started) {
      throw new Error("AudioBufferSourceNode can only be started once");
    }
    this.#started = true;
    // Use -1 to indicate no duration limit
    op_audio_start(this._nodeId, when, offset, duration ?? -1);
  }

  stop(when = 0) {
    if (!this.#started) {
      throw new Error("AudioBufferSourceNode has not been started");
    }
    op_audio_stop(this._nodeId, when);
  }
}

export { AudioBufferSourceNode };
