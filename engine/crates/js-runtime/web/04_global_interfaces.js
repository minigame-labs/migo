import { primordials } from "ext:core/mod.js";
const {
  Symbol,
  SymbolToStringTag,
  TypeError,
} = primordials;
import { EventTarget } from "./02_event.js";

const illegalConstructorKey = Symbol("illegalConstructorKey");

class Window extends EventTarget {
  constructor(key = null) {
    if (key !== illegalConstructorKey) {
      throw new TypeError("Illegal constructor");
    }
    super();
  }

  get [SymbolToStringTag]() {
    return "Window";
  }
}

export {
  Window,
};
