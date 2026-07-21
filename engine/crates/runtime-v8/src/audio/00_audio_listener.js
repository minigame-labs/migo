import { AudioParam } from "ext:host_v8_audio/00_audio_param.js";

/**
 * AudioListener: represents the listener in 3D audio space.
 *
 * Used by PannerNode for spatialization calculations.
 * Position and orientation can be set via AudioParam for automation.
 */
class AudioListener {
  #positionX;
  #positionY;
  #positionZ;
  #forwardX;
  #forwardY;
  #forwardZ;
  #upX;
  #upY;
  #upZ;

  constructor() {
    const MIN = -3.4028235e38;
    const MAX = 3.4028235e38;
    this.#positionX = new AudioParam(0, MIN, MAX);
    this.#positionY = new AudioParam(0, MIN, MAX);
    this.#positionZ = new AudioParam(0, MIN, MAX);
    this.#forwardX = new AudioParam(0, MIN, MAX);
    this.#forwardY = new AudioParam(0, MIN, MAX);
    this.#forwardZ = new AudioParam(-1, MIN, MAX);
    this.#upX = new AudioParam(0, MIN, MAX);
    this.#upY = new AudioParam(1, MIN, MAX);
    this.#upZ = new AudioParam(0, MIN, MAX);
  }

  get positionX() { return this.#positionX; }
  get positionY() { return this.#positionY; }
  get positionZ() { return this.#positionZ; }
  get forwardX() { return this.#forwardX; }
  get forwardY() { return this.#forwardY; }
  get forwardZ() { return this.#forwardZ; }
  get upX() { return this.#upX; }
  get upY() { return this.#upY; }
  get upZ() { return this.#upZ; }

  setPosition(x, y, z) {
    this.#positionX.value = x;
    this.#positionY.value = y;
    this.#positionZ.value = z;
  }

  setOrientation(x, y, z, upX, upY, upZ) {
    this.#forwardX.value = x;
    this.#forwardY.value = y;
    this.#forwardZ.value = z;
    this.#upX.value = upX;
    this.#upY.value = upY;
    this.#upZ.value = upZ;
  }
}

export { AudioListener };
