import { AudioNode } from "ext:host_v8_audio/00_audio_node.js";

class ChannelSplitterNode extends AudioNode {
  constructor(context, nodeId, numberOfOutputs = 6) {
    super(context, nodeId, {
      numberOfInputs: 1,
      numberOfOutputs,
    });
  }
}

export { ChannelSplitterNode };
