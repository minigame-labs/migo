import { primordials } from "ext:core/mod.js";
const { Uint8Array, Uint32Array, TypedArrayPrototypeGetBuffer } = primordials;
import { op_now, op_now_us } from "ext:core/ops";

const hrU8 = new Uint8Array(8);
const hr = new Uint32Array(TypedArrayPrototypeGetBuffer(hrU8));

class BrowserPerformance {
    now() {
        op_now(hrU8);
        return hr[0] * 1000 + hr[1] / 1e6;
    }
}

class Performance {
    now() {
        return op_now_us();
    }
}

export const performance = new BrowserPerformance();

const minigamePerformance = new Performance();
export const getPerformance = () => minigamePerformance;