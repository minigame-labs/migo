import {
  op_audio_set_biquad_filter_type,
  op_audio_get_frequency_response,
} from "ext:core/ops";
import { AudioParam } from "ext:host_v8_audio/00_audio_param.js";
import { AudioNode } from "ext:host_v8_audio/00_audio_node.js";

class BiquadFilterNode extends AudioNode {
  #type = "lowpass";
  #frequency;
  #detune;
  #Q;
  #gain;

  constructor(context, nodeId) {
    super(context, nodeId, {
      numberOfInputs: 1,
      numberOfOutputs: 1,
    });

    this.#frequency = new AudioParam(350.0, 0, 22050);
    this.#frequency._bind(nodeId, "frequency");
    this.#detune = new AudioParam(0, -153600, 153600);
    this.#detune._bind(nodeId, "detune");
    this.#Q = new AudioParam(1.0, 0.0001, 1000.0);
    this.#Q._bind(nodeId, "Q");
    this.#gain = new AudioParam(0, -40, 40);
    this.#gain._bind(nodeId, "gain");
  }

  get type() {
    return this.#type;
  }

  set type(value) {
    const valid = [
      "lowpass", "highpass", "bandpass", "lowshelf",
      "highshelf", "peaking", "notch", "allpass",
    ];
    if (valid.includes(value)) {
      this.#type = value;
      op_audio_set_biquad_filter_type(this._nodeId, value);
    }
  }

  get frequency() {
    return this.#frequency;
  }

  get detune() {
    return this.#detune;
  }

  get Q() {
    return this.#Q;
  }

  get gain() {
    return this.#gain;
  }

  async getFrequencyResponse(frequencyHz, magResponse, phaseResponse) {
    const frequencies = Array.from(frequencyHz);
    const [mag, phase] = await op_audio_get_frequency_response(this._nodeId, frequencies);
    const len = Math.min(frequencyHz.length, mag.length);
    for (let i = 0; i < len; i++) {
      magResponse[i] = mag[i];
      phaseResponse[i] = phase[i];
    }
  }
}

export { BiquadFilterNode };
