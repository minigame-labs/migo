import {
  op_audio_param_set_value_at_time,
  op_audio_param_linear_ramp,
  op_audio_param_exponential_ramp,
  op_audio_param_set_target,
  op_audio_param_cancel_scheduled,
} from "ext:core/ops";

class AudioParam {
  #value;
  #defaultValue;
  #minValue;
  #maxValue;
  // Node ID and param name for native automation dispatch
  #nodeId = null;
  #paramName = null;

  constructor(defaultValue, minValue = -3.4028235e38, maxValue = 3.4028235e38) {
    this.#value = defaultValue;
    this.#defaultValue = defaultValue;
    this.#minValue = minValue;
    this.#maxValue = maxValue;
  }

  // Internal: bind this param to a specific node and param name
  _bind(nodeId, paramName) {
    this.#nodeId = nodeId;
    this.#paramName = paramName;
  }

  get value() {
    return this.#value;
  }

  set value(v) {
    const val = Number(v);
    if (Number.isNaN(val)) return;
    this.#value = Math.max(this.#minValue, Math.min(this.#maxValue, val));
  }

  get defaultValue() {
    return this.#defaultValue;
  }

  get minValue() {
    return this.#minValue;
  }

  get maxValue() {
    return this.#maxValue;
  }

  setValueAtTime(value, startTime) {
    this.#value = value;
    if (this.#nodeId !== null) {
      op_audio_param_set_value_at_time(this.#nodeId, this.#paramName, value, startTime);
    }
    return this;
  }

  linearRampToValueAtTime(value, endTime) {
    this.#value = value;
    if (this.#nodeId !== null) {
      op_audio_param_linear_ramp(this.#nodeId, this.#paramName, value, endTime);
    }
    return this;
  }

  exponentialRampToValueAtTime(value, endTime) {
    this.#value = value;
    if (this.#nodeId !== null) {
      op_audio_param_exponential_ramp(this.#nodeId, this.#paramName, value, endTime);
    }
    return this;
  }

  setTargetAtTime(target, startTime, timeConstant) {
    this.#value = target;
    if (this.#nodeId !== null) {
      op_audio_param_set_target(this.#nodeId, this.#paramName, target, startTime, timeConstant);
    }
    return this;
  }

  cancelScheduledValues(cancelTime) {
    if (this.#nodeId !== null) {
      op_audio_param_cancel_scheduled(this.#nodeId, this.#paramName, cancelTime);
    }
    return this;
  }
}

/**
 * AudioParam with native callback support for immediate value changes.
 * Used by GainNode, etc. where .value assignment must immediately
 * update the native side.
 */
class NativeAudioParam extends AudioParam {
  #onChangeCallback;

  constructor(defaultValue, minValue, maxValue, onChangeCallback) {
    super(defaultValue, minValue, maxValue);
    this.#onChangeCallback = onChangeCallback;
  }

  set value(v) {
    super.value = v;
    if (this.#onChangeCallback) {
      this.#onChangeCallback(super.value);
    }
  }

  get value() {
    return super.value;
  }
}

export { AudioParam, NativeAudioParam };
