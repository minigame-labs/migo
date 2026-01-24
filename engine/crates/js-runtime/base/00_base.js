import { primordials } from "ext:core/mod.js";
const { TypeError, NumberMAX_SAFE_INTEGER, NumberMIN_SAFE_INTEGER, BigInt,
  BigIntAsIntN,
  BigIntAsUintN, Number,
  NumberIsFinite,
  ArrayPrototypePush,
  MathFloor,
  MathMax,
  MathMin,
  MathRound,
  MathTrunc,
  NumberIsNaN,
  ObjectDefineProperty,
  ObjectGetOwnPropertyDescriptors,
  ObjectHasOwn,
  ObjectPrototypeIsPrototypeOf,
  ReflectHas,
  String,
  SymbolIterator,
  SymbolToStringTag,
  StringPrototypeToLowerCase,
  StringPrototypeToUpperCase,
} = primordials;

//  TODO: simplify this
function requiredArguments(length, required, prefix) {
  if (length < required) {
    const errMsg = `${prefix ? prefix + ": " : ""}${required} argument${required === 1 ? "" : "s"
      } required, but only ${length} present`;
    throw new TypeError(errMsg);
  }
}

function makeException(ErrorType, message, prefix, context) {
  return new ErrorType(
    `${prefix ? prefix + ": " : ""}${context ? context : "Value"} ${message}`,
  );
}

function toNumber(value) {
  if (typeof value === "bigint") {
    throw new TypeError("Cannot convert a BigInt value to a number");
  }
  return Number(value);
}

function censorNegativeZero(x) {
  return x === 0 ? 0 : x;
}

function type(V) {
  if (V === null) {
    return "Null";
  }
  switch (typeof V) {
    case "undefined":
      return "Undefined";
    case "boolean":
      return "Boolean";
    case "number":
      return "Number";
    case "string":
      return "String";
    case "symbol":
      return "Symbol";
    case "bigint":
      return "BigInt";
    case "object":
    // Falls through
    case "function":
    // Falls through
    default:
      // Per ES spec, typeof returns an implementation-defined value that is not any of the existing ones for
      // uncallable non-standard exotic objects. Yet Type() which the Web IDL spec depends on returns Object for
      // such cases. So treat the default case as an object.
      return "Object";
  }
}

function integerPart(n) {
  return censorNegativeZero(MathTrunc(n));
}

// Round x to the nearest integer, choosing the even integer if it lies halfway between two.
function evenRound(x) {
  if (
    (x > 0 && x % 1 === +0.5 && (x & 1) === 0) ||
    (x < 0 && x % 1 === -0.5 && (x & 1) === 1)
  ) {
    return censorNegativeZero(MathFloor(x));
  }

  return censorNegativeZero(MathRound(x));
}

function createLongLongConversion(bitLength, { unsigned }) {
  const upperBound = NumberMAX_SAFE_INTEGER;
  const lowerBound = unsigned ? 0 : NumberMIN_SAFE_INTEGER;
  const asBigIntN = unsigned ? BigIntAsUintN : BigIntAsIntN;

  return (
    V,
    prefix = undefined,
    context = undefined,
    opts = { __proto__: null },
  ) => {
    let x = toNumber(V);
    x = censorNegativeZero(x);

    if (opts.enforceRange) {
      if (!NumberIsFinite(x)) {
        throw makeException(
          TypeError,
          "is not a finite number",
          prefix,
          context,
        );
      }

      x = integerPart(x);

      if (x < lowerBound || x > upperBound) {
        throw makeException(
          TypeError,
          `is outside the accepted range of ${lowerBound} to ${upperBound}, inclusive`,
          prefix,
          context,
        );
      }

      return x;
    }

    if (!NumberIsNaN(x) && opts.clamp) {
      x = MathMin(MathMax(x, lowerBound), upperBound);
      x = evenRound(x);
      return x;
    }

    if (!NumberIsFinite(x) || x === 0) {
      return 0;
    }

    let xBigInt = BigInt(integerPart(x));
    xBigInt = asBigIntN(bitLength, xBigInt);
    return Number(xBigInt);
  };
}

const converters = {
  any: (v) => v,
  boolean: (v) => { !!v },
  ["long long"]: createLongLongConversion(64, { unsigned: false }),
  ["unsigned long long"]: createLongLongConversion(64, { unsigned: true }),
  ["DOMString"]: (
    V,
    prefix,
    context,
    opts = { __proto__: null },
  ) => {
    if (typeof V === "string") {
      return V;
    } else if (V === null && opts.treatNullAsEmptyString) {
      return "";
    } else if (typeof V === "symbol") {
      throw makeException(
        TypeError,
        "is a symbol, which cannot be converted to a string",
        prefix,
        context,
      );
    }

    return String(V);
  },
  StringLower: StringPrototypeToLowerCase,
  StringUpper: StringPrototypeToUpperCase,
};


function configureProperties(obj) {
  const descriptors = ObjectGetOwnPropertyDescriptors(obj);
  for (const key in descriptors) {
    if (!ObjectHasOwn(descriptors, key)) {
      continue;
    }
    if (key === "constructor") continue;
    if (key === "prototype") continue;
    const descriptor = descriptors[key];
    if (
      ReflectHas(descriptor, "value") &&
      typeof descriptor.value === "function"
    ) {
      ObjectDefineProperty(obj, key, {
        __proto__: null,
        enumerable: true,
        writable: true,
        configurable: true,
      });
    } else if (ReflectHas(descriptor, "get")) {
      ObjectDefineProperty(obj, key, {
        __proto__: null,
        enumerable: true,
        configurable: true,
      });
    }
  }
}

function configureInterface(interface_) {
  configureProperties(interface_);
  configureProperties(interface_.prototype);
  ObjectDefineProperty(interface_.prototype, SymbolToStringTag, {
    __proto__: null,
    value: interface_.name,
    enumerable: false,
    configurable: true,
    writable: false,
  });
}

function createSequenceConverter(converter) {
  return function (
    V,
    prefix = undefined,
    context = undefined,
    opts = { __proto__: null },
  ) {
    if (type(V) !== "Object") {
      throw makeException(
        TypeError,
        "can not be converted to sequence.",
        prefix,
        context,
      );
    }
    const iter = V?.[SymbolIterator]?.();
    if (iter === undefined) {
      throw makeException(
        TypeError,
        "can not be converted to sequence.",
        prefix,
        context,
      );
    }
    const array = [];
    while (true) {
      const res = iter?.next?.();
      if (res === undefined) {
        throw makeException(
          TypeError,
          "can not be converted to sequence.",
          prefix,
          context,
        );
      }
      if (res.done === true) break;
      const val = converter(
        res.value,
        prefix,
        `${context}, index ${array.length}`,
        opts,
      );
      ArrayPrototypePush(array, val);
    }
    return array;
  };
}

function createInterfaceConverter(name, prototype) {
  return (V, prefix, context, _opts) => {
    if (!ObjectPrototypeIsPrototypeOf(prototype, V)) {
      throw makeException(
        TypeError,
        `is not of type ${name}`,
        prefix,
        context,
      );
    }
    return V;
  };
}

export {
  requiredArguments,
  type,
  converters,
  configureInterface,
  createSequenceConverter,
  createInterfaceConverter,
}