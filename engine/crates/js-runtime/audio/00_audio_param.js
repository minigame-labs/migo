class AudioParam {
  #value;
  #defaultValue;
  #minValue;
  #maxValue;

  constructor(defaultValue, minValue = -3.4028235e38, maxValue = 3.4028235e38) {
    this.#value = defaultValue;
    this.#defaultValue = defaultValue;
    this.#minValue = minValue;
    this.#maxValue = maxValue;
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

  // Automation methods (stubs for now)
  setValueAtTime(value, startTime) {
    this.value = value;
    return this;
  }

  linearRampToValueAtTime(value, endTime) {
    this.value = value;
    return this;
  }

  exponentialRampToValueAtTime(value, endTime) {
    this.value = value;
    return this;
  }

  setTargetAtTime(target, startTime, timeConstant) {
    this.value = target;
    return this;
  }

  cancelScheduledValues(cancelTime) {
    return this;
  }
}

/**
 * AudioParam with native callback support for GainNode etc.
 */
class GainAudioParam extends AudioParam {
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

export { AudioParam, GainAudioParam };
