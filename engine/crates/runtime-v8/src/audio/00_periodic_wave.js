/**
 * PeriodicWave: represents a custom waveform for use with OscillatorNode.
 *
 * Contains the Fourier coefficients (real and imaginary) for the waveform.
 * Used by OscillatorNode when type is "custom".
 */
class PeriodicWave {
  #real;
  #imag;

  constructor(context, options = {}) {
    const real = options.real || new Float32Array([0, 0]);
    const imag = options.imag || new Float32Array([0, 1]);

    if (real.length !== imag.length) {
      throw new Error("real and imag arrays must have the same length");
    }

    this.#real = real instanceof Float32Array ? real : new Float32Array(real);
    this.#imag = imag instanceof Float32Array ? imag : new Float32Array(imag);
  }

  get _real() {
    return this.#real;
  }

  get _imag() {
    return this.#imag;
  }
}

export { PeriodicWave };
