import { AudioParam } from "ext:host_v8_audio/00_audio_param.js";
import { AudioNode } from "ext:host_v8_audio/00_audio_node.js";

class DynamicsCompressorNode extends AudioNode {
  #threshold;
  #knee;
  #ratio;
  #attack;
  #release;

  constructor(context, nodeId) {
    super(context, nodeId, {
      numberOfInputs: 1,
      numberOfOutputs: 1,
    });

    this.#threshold = new AudioParam(-24.0, -100, 0);
    this.#threshold._bind(nodeId, "threshold");
    this.#knee = new AudioParam(30.0, 0, 40);
    this.#knee._bind(nodeId, "knee");
    this.#ratio = new AudioParam(12.0, 1, 20);
    this.#ratio._bind(nodeId, "ratio");
    this.#attack = new AudioParam(0.003, 0, 1);
    this.#attack._bind(nodeId, "attack");
    this.#release = new AudioParam(0.25, 0, 1);
    this.#release._bind(nodeId, "release");
  }

  get threshold() {
    return this.#threshold;
  }

  get knee() {
    return this.#knee;
  }

  get ratio() {
    return this.#ratio;
  }

  get attack() {
    return this.#attack;
  }

  get release() {
    return this.#release;
  }

  get reduction() {
    // Read-only: actual value is computed on native side
    return 0;
  }
}

export { DynamicsCompressorNode };
