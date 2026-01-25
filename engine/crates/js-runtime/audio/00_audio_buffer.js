// AudioBuffer implementation

class AudioBuffer {
  #id;
  #duration;
  #sampleRate;
  #numberOfChannels;
  #length;

  constructor(id, info) {
    this.#id = id;
    this.#duration = info.duration;
    this.#sampleRate = info.sample_rate;
    this.#numberOfChannels = info.channels;
    this.#length = info.length || Math.floor(info.duration * info.sample_rate);
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

  // Not implemented: getChannelData, copyFromChannel, copyToChannel
  getChannelData(channel) {
    throw new Error("AudioBuffer.getChannelData is not implemented");
  }

  copyFromChannel(destination, channelNumber, startInChannel) {
    throw new Error("AudioBuffer.copyFromChannel is not implemented");
  }

  copyToChannel(source, channelNumber, startInChannel) {
    throw new Error("AudioBuffer.copyToChannel is not implemented");
  }
}

export { AudioBuffer };
