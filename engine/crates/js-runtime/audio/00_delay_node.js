import { AudioParam } from "ext:host_v8_audio/00_audio_param.js";
import { AudioNode } from "ext:host_v8_audio/00_audio_node.js";

class DelayNode extends AudioNode {
  #delayTime;

  constructor(context, nodeId, maxDelayTime = 1.0) {
    super(context, nodeId, {
      numberOfInputs: 1,
      numberOfOutputs: 1,
    });

    this.#delayTime = new AudioParam(0, 0, maxDelayTime);
    this.#delayTime._bind(nodeId, "delayTime");
  }

  get delayTime() {
    return this.#delayTime;
  }
}

export { DelayNode };
