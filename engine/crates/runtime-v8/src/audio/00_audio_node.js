import { op_audio_connect, op_audio_disconnect } from "ext:core/ops";

class AudioNode {
  #context;
  #nodeId;
  #connections = [];
  #numberOfInputs;
  #numberOfOutputs;
  #channelCount;
  #channelCountMode;
  #channelInterpretation;

  constructor(context, nodeId, options = {}) {
    this.#context = context;
    this.#nodeId = nodeId;
    this.#numberOfInputs = options.numberOfInputs ?? 1;
    this.#numberOfOutputs = options.numberOfOutputs ?? 1;
    this.#channelCount = options.channelCount ?? 2;
    this.#channelCountMode = options.channelCountMode ?? "max";
    this.#channelInterpretation = options.channelInterpretation ?? "speakers";
  }

  get context() {
    return this.#context;
  }

  get _nodeId() {
    return this.#nodeId;
  }

  get numberOfInputs() {
    return this.#numberOfInputs;
  }

  get numberOfOutputs() {
    return this.#numberOfOutputs;
  }

  get channelCount() {
    return this.#channelCount;
  }

  set channelCount(value) {
    this.#channelCount = value;
  }

  get channelCountMode() {
    return this.#channelCountMode;
  }

  set channelCountMode(value) {
    this.#channelCountMode = value;
  }

  get channelInterpretation() {
    return this.#channelInterpretation;
  }

  set channelInterpretation(value) {
    this.#channelInterpretation = value;
  }

  connect(destination, outputIndex = 0, inputIndex = 0) {
    if (!(destination instanceof AudioNode)) {
      throw new TypeError("destination must be an AudioNode");
    }
    op_audio_connect(this.#nodeId, destination._nodeId);
    this.#connections.push(destination);
    return destination;
  }

  disconnect(destination) {
    op_audio_disconnect(this.#nodeId);
    if (destination) {
      const idx = this.#connections.indexOf(destination);
      if (idx >= 0) {
        this.#connections.splice(idx, 1);
      }
    } else {
      this.#connections.length = 0;
    }
  }
}

class AudioDestinationNode extends AudioNode {
  #maxChannelCount;

  constructor(context, nodeId, maxChannelCount = 2) {
    super(context, nodeId, {
      numberOfInputs: 1,
      numberOfOutputs: 0,
    });
    this.#maxChannelCount = maxChannelCount;
  }

  get maxChannelCount() {
    return this.#maxChannelCount;
  }
}

function validateScheduledTime(value, name = "when") {
  const number = Number(value);
  if (!Number.isFinite(number) || number < 0) {
    throw new RangeError(`${name} must be a finite, non-negative number`);
  }
  return number;
}

function validateFiniteDouble(value, name) {
  const number = Number(value);
  if (!Number.isFinite(number)) {
    throw new TypeError(`${name} must be a finite number`);
  }
  return number;
}

export {
  AudioNode,
  AudioDestinationNode,
  validateScheduledTime,
  validateFiniteDouble,
};
