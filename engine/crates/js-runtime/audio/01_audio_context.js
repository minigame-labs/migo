import {
  op_audio_create_context,
  op_audio_close_context,
  op_audio_decode_audio_data,
  op_audio_create_buffer_source,
  op_audio_create_gain,
  op_audio_create_buffer,
  op_audio_get_channel_data,
  op_audio_get_all_channel_data,
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
import { onHide, onShow } from "ext:host_v8_lifecycle/01_lifecycle.js";

const CONTEXT_REGISTRY = new Map();
const BUFFER_REGISTRY = new Map();

// JS-side node ID generator (starting from 1000 to avoid collision with native IDs)
let nextNodeId = 1000;

// JS-side AudioContext ID generator. Allocated synchronously so a freshly
// constructed context is usable on the very next line (browser semantics -
// games do `new AudioContext().createGain()`), instead of racing an async
// round-trip to the audio thread that leaves `#nativeId` null. Contexts and
// nodes live in separate registries on the audio thread, so this range only
// needs to be internally unique.
let nextContextId = 1;

// PCM allocation limits - mirror of `audio::limits` (the Rust side is the
// authoritative source of truth; native validates again). Kept here so invalid
// createBuffer args fail synchronously with a clear error and never allocate.
const MAX_AUDIO_PCM_BYTES = 512 * 1024 * 1024;
const MAX_AUDIO_CHANNELS = 32;
const MIN_SAMPLE_RATE = 3000;
const MAX_SAMPLE_RATE = 768000;

// While the app is hidden the audio thread is paused (OnHide -> PauseAll), so
// native currentTime freezes. Freeze every context's JS clock to match, and
// resume it on foreground, so scheduled start(when)/stop(when) stay aligned
// with the native timeline across a background/foreground cycle. The
// module-level flag also lets contexts CREATED while backgrounded start frozen.
let _appBackgrounded = false;
onHide(() => {
  _appBackgrounded = true;
  for (const ctx of CONTEXT_REGISTRY.values()) ctx._setBackgrounded(true);
});
onShow(() => {
  _appBackgrounded = false;
  for (const ctx of CONTEXT_REGISTRY.values()) ctx._setBackgrounded(false);
});

class BaseAudioContext {
  #nativeId = null;
  #sampleRate;
  #destination = null;
  #state = "suspended";
  // Clock bookkeeping so `currentTime` mirrors the native frame clock
  // (frames_processed / sampleRate): it starts at 0 when the context is
  // created and freezes whenever native processing stops - both on explicit
  // suspend() and while the app is backgrounded (OnHide pauses the audio
  // thread, freezing native frames_processed, without changing #state).
  #clockEpoch = 0; // performance.now() (ms) at the start of the running segment
  #accumulated = 0; // running seconds banked before the current segment
  #backgrounded = false; // app hidden -> audio thread paused
  #clockRunning = false; // whether the epoch is currently counting

  constructor(sampleRate) {
    this.#sampleRate = sampleRate;
  }

  // Recompute whether the clock should advance ((running state) AND (foreground))
  // and bank/restart the epoch on any change, so `currentTime` stays aligned
  // with native current_time across suspend/resume and background/foreground.
  #reconcileClock() {
    const shouldRun = this.#state === "running" && !this.#backgrounded;
    if (shouldRun === this.#clockRunning) return;
    const now = performance.now();
    if (this.#clockRunning) {
      this.#accumulated += (now - this.#clockEpoch) / 1000; // freezing: bank elapsed
    } else {
      this.#clockEpoch = now; // resuming: restart the epoch
    }
    this.#clockRunning = shouldRun;
  }

  /** Internal: freeze/resume the clock when the app is hidden/shown. */
  _setBackgrounded(backgrounded) {
    this.#backgrounded = !!backgrounded;
    this.#reconcileClock();
  }

  // Synchronous: allocate the id in JS and fire the create command. It rides
  // the same FIFO channel as every subsequent node op, so the audio thread
  // always creates the context before any node that references it. This keeps
  // `new AudioContext()` synchronously usable, matching the browser.
  _initNative(sampleRate) {
    this.#nativeId = nextContextId++;
    op_audio_create_context(this.#nativeId, sampleRate || 0);
    this.#destination = new AudioDestinationNode(this, 0, 2);
    // Anchor the clock origin at creation, matching native frames_processed=0.
    // If the app is already backgrounded, start frozen (native is paused).
    this.#accumulated = 0;
    this.#backgrounded = _appBackgrounded;
    this.#state = "running";
    this.#reconcileClock(); // start counting only if running + foreground
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
    // Sample-frame clock (W3C): starts at 0 at creation, advances in real time
    // while running in the foreground, freezes while suspended/closed/hidden.
    if (this.#clockRunning) {
      return this.#accumulated + (performance.now() - this.#clockEpoch) / 1000;
    }
    return this.#accumulated;
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

      // Fetch all channel data in one round-trip (eliminates per-channel await).
      // Layout: [u32le chCount | u32le framesPerCh | ch0 f32s | ch1 f32s ...]
      const flat = await op_audio_get_all_channel_data(this.#nativeId, info.id);
      const hdr = new DataView(flat.buffer, flat.byteOffset, 8);
      const chCount = hdr.getUint32(0, true);
      const framesPerCh = hdr.getUint32(4, true);
      const dataOffset = flat.byteOffset + 8;
      const bytesPerCh = framesPerCh * 4;
      const channelData = [];
      for (let ch = 0; ch < chCount; ch++) {
        channelData.push(new Float32Array(flat.buffer, dataOffset + ch * bytesPerCh, framesPerCh));
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
    numberOfChannels = Math.trunc(numberOfChannels);
    length = Math.trunc(length);
    sampleRate = Math.trunc(sampleRate);

    if (!Number.isFinite(numberOfChannels) || numberOfChannels < 1 || numberOfChannels > MAX_AUDIO_CHANNELS) {
      throw new Error(
        `createBuffer: numberOfChannels ${numberOfChannels} out of range [1, ${MAX_AUDIO_CHANNELS}]`
      );
    }
    if (!Number.isFinite(length) || length < 1) {
      throw new Error(`createBuffer: length must be a positive integer`);
    }
    if (!Number.isFinite(sampleRate) || sampleRate < MIN_SAMPLE_RATE || sampleRate > MAX_SAMPLE_RATE) {
      throw new Error(
        `createBuffer: sampleRate ${sampleRate} out of range [${MIN_SAMPLE_RATE}, ${MAX_SAMPLE_RATE}]`
      );
    }
    // f32 PCM: length * channels * 4 bytes. Exact in f64 for any valid input.
    const bytes = length * numberOfChannels * 4;
    if (bytes > MAX_AUDIO_PCM_BYTES) {
      throw new Error(
        `createBuffer: ${bytes} bytes exceeds the PCM budget of ${MAX_AUDIO_PCM_BYTES}`
      );
    }

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
    // Keep the sample-frame clock consistent across suspend/resume by
    // recomputing the advancing predicate (see #reconcileClock).
    this.#state = state;
    this.#reconcileClock();
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

    // Initialize synchronously so the context is immediately usable. Expose a
    // resolved promise for callers that `await context.ready`.
    this._initNative(sampleRate);
    this.#ready = Promise.resolve(this);
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
