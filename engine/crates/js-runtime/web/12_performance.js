import { primordials } from "ext:core/mod.js";
const { Uint8Array, Uint32Array, TypedArrayPrototypeGetBuffer } = primordials;
import { op_now } from "ext:core/ops";

const hrU8 = new Uint8Array(8);
const hr = new Uint32Array(TypedArrayPrototypeGetBuffer(hrU8));

class Performance {
    now() {
        op_now(hrU8);
        return hr[0] * 1000 + hr[1] / 1e6;
    }
}

export const performance = new Performance();
