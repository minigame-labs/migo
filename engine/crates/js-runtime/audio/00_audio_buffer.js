// AudioBuffer implementation
//
// Channel data is pre-fetched at construction time (for decoded buffers)
// or zero-filled (for createBuffer). This allows getChannelData() to be
// synchronous as required by the Web Audio API spec.

class AudioBuffer {
  #id;
  #duration;
  #sampleRate;
  #numberOfChannels;
  #length;
  #channelData;

  /**
   * @param {number} id - Native buffer ID
   * @param {object} info - { duration, sample_rate, channels, length }
   * @param {Float32Array[]|null} channelData - Pre-fetched channel data arrays, or null to zero-fill
   */
  constructor(id, info, channelData = null) {
    this.#id = id;
    this.#duration = info.duration;
    this.#sampleRate = info.sample_rate;
    this.#numberOfChannels = info.channels;
    this.#length = info.length || Math.floor(info.duration * info.sample_rate);

    if (channelData) {
      this.#channelData = channelData;
    } else {
      // createBuffer: allocate zero-filled Float32Arrays
      this.#channelData = [];
      for (let i = 0; i < this.#numberOfChannels; i++) {
        this.#channelData.push(new Float32Array(this.#length));
      }
    }
  }

  get _id() {
    return this.#id;
  }

  get duration() {
    return this.#duration;
  }

  get sampleRate() {
    return this.#sampleRate;
  }

  get numberOfChannels() {
    return this.#numberOfChannels;
  }

  get length() {
    return this.#length;
  }

  getChannelData(channel) {
    if (channel < 0 || channel >= this.#numberOfChannels) {
      throw new RangeError(`channel index ${channel} out of range [0, ${this.#numberOfChannels})`);
    }
    return this.#channelData[channel];
  }

  copyFromChannel(destination, channelNumber, startInChannel = 0) {
    if (channelNumber < 0 || channelNumber >= this.#numberOfChannels) {
      throw new RangeError(`channel index ${channelNumber} out of range [0, ${this.#numberOfChannels})`);
    }
    const data = this.#channelData[channelNumber];
    const len = Math.min(destination.length, data.length - startInChannel);
    if (len > 0) {
      destination.set(data.subarray(startInChannel, startInChannel + len));
    }
  }

  copyToChannel(source, channelNumber, startInChannel = 0) {
    if (channelNumber < 0 || channelNumber >= this.#numberOfChannels) {
      throw new RangeError(`channel index ${channelNumber} out of range [0, ${this.#numberOfChannels})`);
    }
    const data = this.#channelData[channelNumber];
    const len = Math.min(source.length, data.length - startInChannel);
    if (len > 0) {
      data.set(source.subarray(0, len), startInChannel);
    }
  }
}

export { AudioBuffer };
