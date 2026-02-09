import { AudioNode } from "ext:host_v8_audio/00_audio_node.js";

/**
 * ScriptProcessorNode (DEPRECATED)
 *
 * This is a stub implementation. The ScriptProcessorNode is deprecated
 * in the Web Audio specification in favor of AudioWorklet.
 * It passes audio through without processing.
 */
class ScriptProcessorNode extends AudioNode {
  #bufferSize;
  #onaudioprocess = null;

  constructor(context, nodeId, bufferSize, numberOfInputChannels, numberOfOutputChannels) {
    super(context, nodeId, {
      numberOfInputs: 1,
      numberOfOutputs: 1,
    });
    this.#bufferSize = bufferSize;
    console.warn(
      "ScriptProcessorNode is deprecated. Use AudioWorkletNode instead."
    );
  }

  get bufferSize() {
    return this.#bufferSize;
  }

  get onaudioprocess() {
    return this.#onaudioprocess;
  }

  set onaudioprocess(value) {
    this.#onaudioprocess = typeof value === "function" ? value : null;
  }
}

export { ScriptProcessorNode };
