import {
  op_audio_set_wave_shaper_curve,
  op_audio_set_wave_shaper_oversample,
} from "ext:core/ops";
import { AudioNode } from "ext:host_v8_audio/00_audio_node.js";

class WaveShaperNode extends AudioNode {
  #curve = null;
  #oversample = "none";

  constructor(context, nodeId) {
    super(context, nodeId, {
      numberOfInputs: 1,
      numberOfOutputs: 1,
    });
  }

  get curve() {
    return this.#curve;
  }

  set curve(value) {
    if (value === null) {
      this.#curve = null;
      op_audio_set_wave_shaper_curve(this._nodeId, null);
    } else if (value instanceof Float32Array) {
      this.#curve = value;
      // Send raw bytes of the Float32Array to native
      op_audio_set_wave_shaper_curve(
        this._nodeId,
        new Uint8Array(value.buffer, value.byteOffset, value.byteLength)
      );
    }
  }

  get oversample() {
    return this.#oversample;
  }

  set oversample(value) {
    const valid = ["none", "2x", "4x"];
    if (valid.includes(value)) {
      this.#oversample = value;
      op_audio_set_wave_shaper_oversample(this._nodeId, value);
    }
  }
}

export { WaveShaperNode };
