import { op_audio_set_gain_value } from "ext:core/ops";
import { NativeAudioParam } from "ext:host_v8_audio/00_audio_param.js";
import { AudioNode } from "ext:host_v8_audio/00_audio_node.js";

class GainNode extends AudioNode {
  #gain;

  constructor(context, nodeId) {
    super(context, nodeId, {
      numberOfInputs: 1,
      numberOfOutputs: 1,
    });

    // Create gain AudioParam with native sync callback
    this.#gain = new NativeAudioParam(1.0, -3.4028235e38, 3.4028235e38, (value) => {
      op_audio_set_gain_value(nodeId, value);
    });
    // Bind for automation dispatch
    this.#gain._bind(nodeId, "gain");
  }

  get gain() {
    return this.#gain;
  }
}

export { GainNode };
