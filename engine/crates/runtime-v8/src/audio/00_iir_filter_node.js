import { op_audio_get_frequency_response } from "ext:core/ops";
import { AudioNode } from "ext:host_v8_audio/00_audio_node.js";

class IIRFilterNode extends AudioNode {
  #feedforward;
  #feedback;

  constructor(context, nodeId, feedforward, feedback) {
    super(context, nodeId, {
      numberOfInputs: 1,
      numberOfOutputs: 1,
    });
    this.#feedforward = feedforward;
    this.#feedback = feedback;
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

export { IIRFilterNode };
