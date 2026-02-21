import {
  op_audio_create_context,
  op_audio_close_context,
  op_audio_decode_audio_data,
  op_audio_create_buffer_source,
  op_audio_create_gain,
  op_audio_create_buffer,
  op_audio_get_channel_data,
  op_audio_create_oscillator,
  op_audio_create_delay,
  op_audio_create_biquad_filter,
  op_audio_create_wave_shaper,
  op_audio_create_analyser,
  op_audio_create_dynamics_compressor,
  op_audio_create_panner,
  op_audio_create_channel_merger,
  op_audio_create_channel_splitter,
  op_audio_create_constant_source,
  op_audio_create_iir_filter,
  op_audio_resume_context,
  op_audio_suspend_context,
} from "ext:core/ops";
import { AudioParam } from "ext:host_v8_audio/00_audio_param.js";
import { AudioBuffer } from "ext:host_v8_audio/00_audio_buffer.js";
import { AudioNode, AudioDestinationNode } from "ext:host_v8_audio/00_audio_node.js";
import { AudioBufferSourceNode } from "ext:host_v8_audio/00_buffer_source_node.js";
import { GainNode } from "ext:host_v8_audio/00_gain_node.js";
import { OscillatorNode } from "ext:host_v8_audio/00_oscillator_node.js";
import { DelayNode } from "ext:host_v8_audio/00_delay_node.js";
import { BiquadFilterNode } from "ext:host_v8_audio/00_biquad_filter_node.js";
import { WaveShaperNode } from "ext:host_v8_audio/00_wave_shaper_node.js";
import { AnalyserNode } from "ext:host_v8_audio/00_analyser_node.js";
import { DynamicsCompressorNode } from "ext:host_v8_audio/00_dynamics_compressor_node.js";
import { PannerNode } from "ext:host_v8_audio/00_panner_node.js";
import { ChannelMergerNode } from "ext:host_v8_audio/00_channel_merger_node.js";
import { ChannelSplitterNode } from "ext:host_v8_audio/00_channel_splitter_node.js";
import { ConstantSourceNode } from "ext:host_v8_audio/00_constant_source_node.js";
import { IIRFilterNode } from "ext:host_v8_audio/00_iir_filter_node.js";
import { ScriptProcessorNode } from "ext:host_v8_audio/00_script_processor_node.js";
import { PeriodicWave } from "ext:host_v8_audio/00_periodic_wave.js";
import { AudioListener } from "ext:host_v8_audio/00_audio_listener.js";

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

      // Eagerly fetch all channel data so getChannelData() can be synchronous
      const channelData = [];
      for (let ch = 0; ch < info.channels; ch++) {
        const bytes = await op_audio_get_channel_data(this.#nativeId, info.id, ch);
        channelData.push(new Float32Array(bytes.buffer, bytes.byteOffset, bytes.byteLength / 4));
      }

      const buffer = new AudioBuffer(info.id, this.#nativeId, info, channelData);
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

  async createBuffer(numberOfChannels, length, sampleRate) {
    const info = await op_audio_create_buffer(
      this.#nativeId,
      numberOfChannels,
      length,
      sampleRate
    );
    // createBuffer: zero-filled arrays allocated on JS side (channelData = null)
    const buffer = new AudioBuffer(info.id, this.#nativeId, info);
    BUFFER_REGISTRY.set(info.id, buffer);
    return buffer;
  }

  createBufferSource() {
    // Generate ID in JS, notify native asynchronously
    const nodeId = nextNodeId++;
    // Fire and forget - native will create the node
    op_audio_create_buffer_source(this.#nativeId, nodeId);
    return new AudioBufferSourceNode(this, nodeId);
  }

  createGain() {
    const nodeId = nextNodeId++;
    op_audio_create_gain(this.#nativeId, nodeId);
    return new GainNode(this, nodeId);
  }

  createOscillator() {
    const nodeId = nextNodeId++;
    op_audio_create_oscillator(this.#nativeId, nodeId);
    return new OscillatorNode(this, nodeId);
  }

  createDelay(maxDelayTime = 1.0) {
    const nodeId = nextNodeId++;
    op_audio_create_delay(this.#nativeId, nodeId, maxDelayTime);
    return new DelayNode(this, nodeId, maxDelayTime);
  }

  createBiquadFilter() {
    const nodeId = nextNodeId++;
    op_audio_create_biquad_filter(this.#nativeId, nodeId);
    return new BiquadFilterNode(this, nodeId);
  }

  createWaveShaper() {
    const nodeId = nextNodeId++;
    op_audio_create_wave_shaper(this.#nativeId, nodeId);
    return new WaveShaperNode(this, nodeId);
  }

  createAnalyser() {
    const nodeId = nextNodeId++;
    op_audio_create_analyser(this.#nativeId, nodeId);
    return new AnalyserNode(this, nodeId);
  }

  createDynamicsCompressor() {
    const nodeId = nextNodeId++;
    op_audio_create_dynamics_compressor(this.#nativeId, nodeId);
    return new DynamicsCompressorNode(this, nodeId);
  }

  createPanner() {
    const nodeId = nextNodeId++;
    op_audio_create_panner(this.#nativeId, nodeId);
    return new PannerNode(this, nodeId);
  }

  createChannelMerger(numberOfInputs = 6) {
    const nodeId = nextNodeId++;
    op_audio_create_channel_merger(this.#nativeId, nodeId, numberOfInputs);
    return new ChannelMergerNode(this, nodeId, numberOfInputs);
  }

  createChannelSplitter(numberOfOutputs = 6) {
    const nodeId = nextNodeId++;
    op_audio_create_channel_splitter(this.#nativeId, nodeId, numberOfOutputs);
    return new ChannelSplitterNode(this, nodeId, numberOfOutputs);
  }

  createConstantSource() {
    const nodeId = nextNodeId++;
    op_audio_create_constant_source(this.#nativeId, nodeId);
    return new ConstantSourceNode(this, nodeId);
  }

  createIIRFilter(feedforward, feedback) {
    const nodeId = nextNodeId++;
    op_audio_create_iir_filter(this.#nativeId, nodeId, feedforward, feedback);
    return new IIRFilterNode(this, nodeId, feedforward, feedback);
  }

  createScriptProcessor(bufferSize = 0, numberOfInputChannels = 2, numberOfOutputChannels = 2) {
    const nodeId = nextNodeId++;
    return new ScriptProcessorNode(this, nodeId, bufferSize, numberOfInputChannels, numberOfOutputChannels);
  }

  createPeriodicWave(options = {}) {
    const real = options.real || undefined;
    const imag = options.imag || undefined;
    const disableNormalization = options.disableNormalization || false;
    return new PeriodicWave({ real, imag, disableNormalization });
  }

  get listener() {
    if (!this._listener) {
      this._listener = new AudioListener(this);
    }
    return this._listener;
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
      throw new Error("Cannot resume a closed AudioContext");
    }
    await op_audio_resume_context(this._nativeId);
    this._setState("running");
  }

  async suspend() {
    if (this.state === "closed") {
      throw new Error("Cannot suspend a closed AudioContext");
    }
    await op_audio_suspend_context(this._nativeId);
    this._setState("suspended");
  }
}

function createWebAudioContext(options) {
  return new AudioContext(options);
}

export {
  createWebAudioContext,
  AudioContext,
  BaseAudioContext,
  AudioBuffer,
  AudioNode,
  AudioDestinationNode,
  AudioBufferSourceNode,
  GainNode,
  OscillatorNode,
  DelayNode,
  BiquadFilterNode,
  WaveShaperNode,
  AnalyserNode,
  DynamicsCompressorNode,
  PannerNode,
  ChannelMergerNode,
  ChannelSplitterNode,
  ConstantSourceNode,
  IIRFilterNode,
  ScriptProcessorNode,
  PeriodicWave,
  AudioListener,
  AudioParam,
};
