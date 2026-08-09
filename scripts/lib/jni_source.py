#!/usr/bin/env python3
"""Reading the Android JNI boundary's two sides as source, for the gates that
compare them.

Two contract gates cross-check the same three files -- `profile_contract.rs`'s
descriptor tables, `inbound.rs`'s handlers, and the Java classes on the other
side -- and both need the same two things: a way to ignore comments and literals
while scanning structure, and the descriptor tables themselves. Kept here rather
than copied into each gate, because a parser bug fixed in one copy and not the
other is the failure that leaves a gate quietly weaker than the one beside it.

Nothing here decides anything. The gates own their rules; this owns only the
reading.
"""

from __future__ import annotations

import re

# --------------------------------------------------------------------- masking
#
# Structural scanning -- matching a call's parentheses, splitting an argument
# list at top-level commas -- has to ignore anything inside a comment or a
# literal. A javadoc line naming a method is not a call site, and `")"` is not a
# closing parenthesis. Blanking rather than deleting keeps every offset and line
# number equal to the original, so text is still read out of the real source and
# reported at its real line.
#
# One masker per language, because the hazards differ. Rust's are not
# hypothetical in the file these gates read: lifetime annotations, which a
# char-literal rule would each take for an opening quote and blank the parameter
# list behind, and raw strings, whose interior escapes mean nothing.


def _blank(out: list[str], start: int, end: int) -> None:
    """Blank `out[start:end]`, leaving newlines so line numbers survive."""
    for index in range(start, end):
        if out[index] != "\n":
            out[index] = " "


def _mask_quoted(source: str, out: list[str], open_quote: int, quote: str) -> int:
    """Blank a backslash-escaped literal's interior; return the offset past it."""
    index, length = open_quote + 1, len(source)
    while index < length:
        if source[index] == "\\":
            _blank(out, index, min(index + 2, length))
            index += 2
            continue
        if source[index] == quote:
            return index + 1
        _blank(out, index, index + 1)
        index += 1
    return length


def mask_java(source: str) -> str:
    """`source` with Java comment and literal bodies blanked, offsets preserved."""
    out = list(source)
    index, length = 0, len(source)
    while index < length:
        if source.startswith("//", index):
            end = source.find("\n", index)
            end = length if end == -1 else end
            _blank(out, index, end)
            index = end
            continue
        if source.startswith("/*", index):
            end = source.find("*/", index + 2)
            end = length if end == -1 else end + 2
            _blank(out, index, end)
            index = end
            continue
        if source[index] in ('"', "'"):
            index = _mask_quoted(source, out, index, source[index])
            continue
        index += 1
    return "".join(out)


# A char literal, and not a lifetime: `'a'`, `'\n'`, `'\u{1f}'`. A lifetime has
# no closing quote, so requiring one is the whole distinction.
_RUST_CHAR = re.compile(r"'(?:\\(?:u\{[0-9a-fA-F]{1,6}\}|x[0-9a-fA-F]{2}|.)|[^\\'])'")
# `r"..."`, `r#"..."#`, `br##"..."##`: the hash count picks the terminator.
_RUST_RAW = re.compile(r"b?r(#*)\"")
_IDENT_CHAR = re.compile(r"[0-9A-Za-z_]")


def mask_rust(source: str) -> str:
    """`source` with Rust comment and literal bodies blanked, offsets preserved."""
    out = list(source)
    index, length = 0, len(source)
    while index < length:
        if source.startswith("//", index):
            end = source.find("\n", index)
            end = length if end == -1 else end
            _blank(out, index, end)
            index = end
            continue
        if source.startswith("/*", index):
            # Rust nests block comments, so a `/*` inside one is not text.
            depth, cursor = 0, index
            while cursor < length:
                if source.startswith("/*", cursor):
                    depth += 1
                    cursor += 2
                elif source.startswith("*/", cursor):
                    depth -= 1
                    cursor += 2
                    if depth == 0:
                        break
                else:
                    cursor += 1
            _blank(out, index, cursor)
            index = cursor
            continue
        raw = _RUST_RAW.match(source, index)
        if raw is not None and not (index and _IDENT_CHAR.match(source[index - 1])):
            terminator = '"' + "#" * len(raw.group(1))
            end = source.find(terminator, raw.end())
            end = length if end == -1 else end + len(terminator)
            _blank(out, raw.end(), end - len(terminator))
            index = end
            continue
        if source[index] == '"':
            index = _mask_quoted(source, out, index, '"')
            continue
        if source[index] == "'":
            literal = _RUST_CHAR.match(source, index)
            if literal is None:
                index += 1  # a lifetime
                continue
            _blank(out, index + 1, literal.end() - 1)
            index = literal.end()
            continue
        index += 1
    return "".join(out)


# ------------------------------------------------------------------ structure


def balanced_end(masked: str, open_bracket: int) -> int | None:
    """Offset of the bracket closing the one at `open_bracket`, or None."""
    depth = 0
    for index in range(open_bracket, len(masked)):
        char = masked[index]
        if char in "([{":
            depth += 1
        elif char in ")]}":
            depth -= 1
            if depth == 0:
                return index
    return None


def split_arguments(text: str, masked: str) -> list[str]:
    """`text` split at its top-level commas, nesting read from `masked`."""
    parts, depth, start = [], 0, 0
    for index, char in enumerate(masked):
        if char in "([{":
            depth += 1
        elif char in ")]}":
            depth -= 1
        elif char == "," and depth == 0:
            parts.append(text[start:index])
            start = index + 1
    parts.append(text[start:])
    return parts


def normalise(text: str) -> str:
    """`text` with its whitespace removed.

    Argument lists wrap across lines, so an argument arrives carrying newlines
    and indentation. Comparing that against an expected spelling without
    normalising would report the formatting as a violation.
    """
    return re.sub(r"\s+", "", text)


def spaced(text: str) -> str:
    """`text` with each run of whitespace collapsed to one space."""
    return re.sub(r"\s+", " ", text)


def line_of(source: str, offset: int) -> int:
    return source.count("\n", 0, offset) + 1


# ---------------------------------------------------------------- descriptors

_ENTRY = re.compile(r'\(\s*"([A-Za-z0-9_]+)"\s*,\s*"([^"]+)"\s*\)')


def descriptor_table(contract_source: str, prefix: str) -> dict[str, str]:
    """`{method: descriptor}` for every `methods![...]` table named `<prefix>*`.

    `prefix` is `NATIVE_` for the Java-to-native direction or `JAVA_` for the
    other one. Read out of the tables rather than from a list, so a method added
    to a profile is covered without editing a gate.
    """
    found: dict[str, str] = {}
    for block in re.finditer(
        rf"const\s+{re.escape(prefix)}[A-Z_]+\s*:\s*&\[JniMethod\]\s*=\s*methods!\[(.*?)\n\];",
        contract_source,
        re.S,
    ):
        for entry in _ENTRY.finditer(block.group(1)):
            found[entry.group(1)] = entry.group(2)
    return found


# JNI's type encoding. `V` is only legal as a return type, which the caller
# enforces by where it looks for it.
_PRIMITIVES = {
    "Z": "boolean",
    "B": "byte",
    "C": "char",
    "S": "short",
    "I": "int",
    "J": "long",
    "F": "float",
    "D": "double",
    "V": "void",
}


def decode_descriptor(descriptor: str) -> tuple[list[str], str]:
    """A JNI descriptor as `([parameter type, ...], return type)` in Java spelling.

    Reference types come back as their *simple* name -- `Ljava/lang/String;` is
    `String` -- because that is how the Java source spells them under its imports,
    and the comparison this feeds is between a descriptor and a declaration.

    Raises ValueError on anything it cannot decode, rather than returning a
    partial answer a caller would compare against and pass.
    """
    if not descriptor.startswith("("):
        raise ValueError(f"descriptor {descriptor!r} does not start with '('")
    close = descriptor.index(")")
    parameters = _decode_sequence(descriptor[1:close])
    returns = _decode_sequence(descriptor[close + 1 :])
    if len(returns) != 1:
        raise ValueError(f"descriptor {descriptor!r} has {len(returns)} return types")
    return parameters, returns[0]


def _decode_sequence(text: str) -> list[str]:
    types: list[str] = []
    index, length = 0, len(text)
    while index < length:
        arrays = 0
        while index < length and text[index] == "[":
            arrays += 1
            index += 1
        if index >= length:
            raise ValueError(f"{text!r} ends in an array marker with no element type")
        char = text[index]
        if char == "L":
            end = text.find(";", index)
            if end == -1:
                raise ValueError(f"unterminated reference type in {text!r}")
            types.append(text[index + 1 : end].rsplit("/", 1)[-1] + "[]" * arrays)
            index = end + 1
            continue
        if char not in _PRIMITIVES:
            raise ValueError(f"unknown type code {char!r} in {text!r}")
        types.append(_PRIMITIVES[char] + "[]" * arrays)
        index += 1
    return types
