import { AudioNode } from "ext:host_v8_audio/00_audio_node.js";

class IIRFilterNode extends AudioNode {
  constructor(context, nodeId) {
    super(context, nodeId, {
      numberOfInputs: 1,
      numberOfOutputs: 1,
    });
  }

  getFrequencyResponse(frequencyHz, magResponse, phaseResponse) {
    // Stub: full implementation requires native-side coefficient access
    const len = frequencyHz.length;
    for (let i = 0; i < len; i++) {
      magResponse[i] = 1.0;
      phaseResponse[i] = 0.0;
    }
  }
}

export { IIRFilterNode };
