import {
  op_audio_create_context,
  op_audio_close_context,
  op_audio_decode_audio_data,
  op_audio_create_buffer_source,
  op_audio_create_gain,
} from "ext:core/ops";
import { AudioParam } from "ext:host_v8_audio/00_audio_param.js";
import { AudioBuffer } from "ext:host_v8_audio/00_audio_buffer.js";
import { AudioNode, AudioDestinationNode } from "ext:host_v8_audio/00_audio_node.js";
import { AudioBufferSourceNode } from "ext:host_v8_audio/00_buffer_source_node.js";
import { GainNode } from "ext:host_v8_audio/00_gain_node.js";

const CONTEXT_REGISTRY = new Map();
const BUFFER_REGISTRY = new Map();

// JS-side node ID generator (starting from 1000 to avoid collision with native IDs)
let nextNodeId = 1000;

class BaseAudioContext {
  #nativeId = null;
  #sampleRate;
  #destination = null;
  #state = "suspended";

  constructor(sampleRate) {
    this.#sampleRate = sampleRate;
  }

  async _initNative(sampleRate) {
    this.#nativeId = await op_audio_create_context(sampleRate || 0);
    this.#destination = new AudioDestinationNode(this, 0, 2);
    this.#state = "running";
    CONTEXT_REGISTRY.set(this.#nativeId, this);
  }

  get _nativeId() {
    return this.#nativeId;
  }

  get sampleRate() {
    return this.#sampleRate;
  }

  get destination() {
    return this.#destination;
  }

  get state() {
    return this.#state;
  }

  get currentTime() {
    // Use high-resolution timer
    return performance.now() / 1000;
  }

  async decodeAudioData(audioData, successCallback, errorCallback) {
    try {
      if (!(audioData instanceof ArrayBuffer)) {
        throw new TypeError("audioData must be an ArrayBuffer");
      }

      const info = await op_audio_decode_audio_data(
        this.#nativeId,
        new Uint8Array(audioData)
      );

      const buffer = new AudioBuffer(info.id, info);
      BUFFER_REGISTRY.set(info.id, buffer);

      if (successCallback) {
        successCallback(buffer);
      }
      return buffer;
    } catch (error) {
      if (errorCallback) {
        errorCallback(error);
        return;
      }
      throw error;
    }
  }

  createBufferSource() {
    // Generate ID in JS, notify native asynchronously
    const nodeId = nextNodeId++;
    // Fire and forget - native will create the node
    op_audio_create_buffer_source(this.#nativeId, nodeId);
    return new AudioBufferSourceNode(this, nodeId);
  }

  createGain() {
    // Generate ID in JS, notify native asynchronously
    const nodeId = nextNodeId++;
    // Fire and forget - native will create the node
    op_audio_create_gain(this.#nativeId, nodeId);
    return new GainNode(this, nodeId);
  }

  _setState(state) {
    this.#state = state;
  }

  _setSampleRate(rate) {
    this.#sampleRate = rate;
  }
}

class AudioContext extends BaseAudioContext {
  #baseLatency = 0.005;
  #outputLatency = 0.01;
  #ready;

  constructor(options = {}) {
    const sampleRate = options.sampleRate || 44100;
    super(sampleRate);

    // Start async initialization
    this.#ready = this._initNative(sampleRate);
  }

  get baseLatency() {
    return this.#baseLatency;
  }

  get outputLatency() {
    return this.#outputLatency;
  }

  get ready() {
    return this.#ready;
  }

  async close() {
    if (this.state === "closed") {
      return;
    }
    await op_audio_close_context(this._nativeId);
    this._setState("closed");
    CONTEXT_REGISTRY.delete(this._nativeId);
  }

  async resume() {
    if (this.state === "closed") {
      throw new DOMException(
        "Cannot resume a closed AudioContext",
        "InvalidStateError"
      );
    }
    // TODO: call native resume
    this._setState("running");
  }

  async suspend() {
    if (this.state === "closed") {
      throw new DOMException(
        "Cannot suspend a closed AudioContext",
        "InvalidStateError"
      );
    }
    // TODO: call native suspend
    this._setState("suspended");
  }
}

export {
  AudioContext,
  BaseAudioContext,
  AudioBuffer,
  AudioNode,
  AudioDestinationNode,
  AudioBufferSourceNode,
  GainNode,
  AudioParam,
};
