import { AudioNode } from "ext:host_v8_audio/00_audio_node.js";

class ChannelMergerNode extends AudioNode {
  constructor(context, nodeId, numberOfInputs = 6) {
    super(context, nodeId, {
      numberOfInputs,
      numberOfOutputs: 1,
    });
  }
}

export { ChannelMergerNode };
