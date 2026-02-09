import {
  op_audio_set_panning_model,
  op_audio_set_distance_model,
  op_audio_set_panner_scalar,
} from "ext:core/ops";
import { AudioParam } from "ext:host_v8_audio/00_audio_param.js";
import { AudioNode } from "ext:host_v8_audio/00_audio_node.js";

class PannerNode extends AudioNode {
  #panningModel = "equalpower";
  #distanceModel = "inverse";
  #positionX;
  #positionY;
  #positionZ;
  #orientationX;
  #orientationY;
  #orientationZ;
  #refDistance = 1;
  #maxDistance = 10000;
  #rolloffFactor = 1;
  #coneInnerAngle = 360;
  #coneOuterAngle = 360;
  #coneOuterGain = 0;

  constructor(context, nodeId) {
    super(context, nodeId, {
      numberOfInputs: 1,
      numberOfOutputs: 1,
    });

    const MIN = -3.4028235e38;
    const MAX = 3.4028235e38;
    this.#positionX = new AudioParam(0, MIN, MAX);
    this.#positionX._bind(nodeId, "positionX");
    this.#positionY = new AudioParam(0, MIN, MAX);
    this.#positionY._bind(nodeId, "positionY");
    this.#positionZ = new AudioParam(0, MIN, MAX);
    this.#positionZ._bind(nodeId, "positionZ");
    this.#orientationX = new AudioParam(1, MIN, MAX);
    this.#orientationX._bind(nodeId, "orientationX");
    this.#orientationY = new AudioParam(0, MIN, MAX);
    this.#orientationY._bind(nodeId, "orientationY");
    this.#orientationZ = new AudioParam(0, MIN, MAX);
    this.#orientationZ._bind(nodeId, "orientationZ");
  }

  get panningModel() { return this.#panningModel; }
  set panningModel(v) {
    if (v === "equalpower" || v === "HRTF") {
      this.#panningModel = v;
      op_audio_set_panning_model(this._nodeId, v);
    }
  }

  get distanceModel() { return this.#distanceModel; }
  set distanceModel(v) {
    if (v === "linear" || v === "inverse" || v === "exponential") {
      this.#distanceModel = v;
      op_audio_set_distance_model(this._nodeId, v);
    }
  }

  get positionX() { return this.#positionX; }
  get positionY() { return this.#positionY; }
  get positionZ() { return this.#positionZ; }
  get orientationX() { return this.#orientationX; }
  get orientationY() { return this.#orientationY; }
  get orientationZ() { return this.#orientationZ; }

  get refDistance() { return this.#refDistance; }
  set refDistance(v) {
    this.#refDistance = Number(v);
    op_audio_set_panner_scalar(this._nodeId, "refDistance", this.#refDistance);
  }

  get maxDistance() { return this.#maxDistance; }
  set maxDistance(v) {
    this.#maxDistance = Number(v);
    op_audio_set_panner_scalar(this._nodeId, "maxDistance", this.#maxDistance);
  }

  get rolloffFactor() { return this.#rolloffFactor; }
  set rolloffFactor(v) {
    this.#rolloffFactor = Number(v);
    op_audio_set_panner_scalar(this._nodeId, "rolloffFactor", this.#rolloffFactor);
  }

  get coneInnerAngle() { return this.#coneInnerAngle; }
  set coneInnerAngle(v) {
    this.#coneInnerAngle = Number(v);
    op_audio_set_panner_scalar(this._nodeId, "coneInnerAngle", this.#coneInnerAngle);
  }

  get coneOuterAngle() { return this.#coneOuterAngle; }
  set coneOuterAngle(v) {
    this.#coneOuterAngle = Number(v);
    op_audio_set_panner_scalar(this._nodeId, "coneOuterAngle", this.#coneOuterAngle);
  }

  get coneOuterGain() { return this.#coneOuterGain; }
  set coneOuterGain(v) {
    this.#coneOuterGain = Number(v);
    op_audio_set_panner_scalar(this._nodeId, "coneOuterGain", this.#coneOuterGain);
  }

  setPosition(x, y, z) {
    this.#positionX.value = x;
    this.#positionY.value = y;
    this.#positionZ.value = z;
  }

  setOrientation(x, y, z) {
    this.#orientationX.value = x;
    this.#orientationY.value = y;
    this.#orientationZ.value = z;
  }
}

export { PannerNode };
