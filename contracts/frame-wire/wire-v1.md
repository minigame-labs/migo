# WireFramePacket v1

The only specification of the bytes that cross a process boundary between a
Migo frame producer and a Migo renderer. Two implementations exist — a
JavaScript encoder running inside WebKit's WebContent process, and the Rust
reader in `engine/crates/frame-wire` — and neither is the specification.

Both are checked against the fixed corpus in `golden/`. Checking them against
each other instead would let a shared misreading pass: two encoders that agree
with one another and not with a fixed corpus is exactly the failure a corpus
exists to catch.

## Conventions

- Little-endian. Every multi-byte field.
- Offsets are byte offsets from the first byte of the packet.
- **No alignment may be assumed of the packet's own base address.** It arrives
  on the reading side inside `Data.withUnsafeBytes`, which promises nothing, so
  a reader must not cast a pointer to a multi-byte type. Section *payloads* are
  8-byte aligned relative to the packet start, which is the guarantee a reader
  can use once it has copied or confirmed the base alignment itself.

## Header — 64 bytes, fixed

| Offset | Size | Field | Rule |
|---:|---:|---|---|
| 0 | 4 | `magic` | exactly `0x4D475046` |
| 4 | 4 | `wire_version` | exactly `1` for this document |
| 8 | 4 | `header_bytes` | exactly `64`. Validated, not trusted: a header that could announce its own length would let a producer move the section table |
| 12 | 4 | `total_bytes` | `>= 64`, `<= 67108864`, and **equal to the delivered byte count** |
| 16 | 8 | `session_nonce` | random per session; a packet with the wrong one is not this session's |
| 24 | 8 | `sequence` | strictly increasing per runtime generation |
| 32 | 4 | `runtime_generation` | the producer generation these bytes were built by |
| 36 | 4 | `surface_generation` | the surface they were built against; `0` means "not surface-bound" |
| 40 | 4 | `resource_epoch` | the resource-table epoch the ids inside are valid in |
| 44 | 4 | `frame_id` | producer's frame counter; advisory, for correlation |
| 48 | 4 | `flags` | see below; unknown bits are fatal |
| 52 | 4 | `section_count` | `<= 8` |
| 56 | 4 | `payload_checksum` | CRC32 of the whole packet with these four bytes read as zero |
| 60 | 4 | `reserved0` | zero |

### Flags

| Bit | Name | Meaning |
|---:|---|---|
| 0 | `PRESENT` | this packet ends a frame; present after executing it |
| 1 | `CONTINUED` | more packets belong to this `frame_id` |

Any other bit set is a rejection. A reader that ignored unknown flags would
execute a frame under semantics it does not implement.

## Section table

`section_count` entries of 16 bytes each, starting at offset 64.

| Offset | Size | Field |
|---:|---:|---|
| 0 | 4 | `kind` |
| 4 | 4 | `offset` |
| 8 | 4 | `byte_length` |
| 12 | 4 | `item_count` |

Rules:

1. `offset % 8 == 0`.
2. `offset >= 64 + 16 * section_count`.
3. `offset + byte_length <= total_bytes`, computed with checked arithmetic. The
   unchecked form wraps, and a wrapped end compares as "fits".
4. Sections are **ascending and disjoint**. Overlap would let one byte range be
   read under two kinds, which the consumer cannot defend against on its own.
5. No kind appears twice.

## Section kinds

| Value | Name | Required | Contents |
|---:|---|---|---|
| 1 | `COMMAND_STREAM` | yes | `u32` words: the drawing commands. `byte_length % 4 == 0`, and `item_count <= byte_length / 4` because a record is at least one word |
| 2 | `INLINE_DATA` | no | small blobs referenced by the command stream |
| 3 | `RESOURCE_REFERENCES` | no | ids of resources uploaded on the resource lane |
| `0x80000001` | `DAMAGE` | no | advisory damage rectangles |
| `0x80000002` | `TIMING` | no | advisory producer timestamps |

Every packet must carry a `COMMAND_STREAM`.

### Forward compatibility

Kinds with bit 31 set are **advisory**: a reader that does not recognise one
skips it. Every other unrecognised kind is a rejection.

The asymmetry is the point. A future advisory section — a new timing channel, a
new hint — costs an old reader nothing to ignore. A future *required* section
carries something the frame depends on, and a reader that skipped it would draw
a frame missing the resource binding it was told about, silently. Adding a
required section is a `wire_version` bump.

## Command stream

The `COMMAND_STREAM` payload is validated separately, by the pure `&[u32]`
structural validator the WebGL path already uses (record header: low 12 bits
opcode, high 20 bits word count). This layer does not know which opcodes exist,
and that is deliberate: envelope correctness must not need updating every time
an opcode is added.

## Checksum

CRC32 (IEEE) over the entire packet, with `payload_checksum`'s own four bytes
substituted with zero.

Covering the header — not only the payload — is what catches the corruption
that still parses. A flipped `frame_id` or `section_count` leaves a
structurally perfect packet; nothing but a checksum over the header notices.

## Rejection codes

Stable, and carried across the C ABI into host telemetry. Never renumber; only
append. The list lives in `engine/crates/frame-wire/src/lib.rs` (`WireError`)
and `src/ingress.rs` (`INGRESS_ERROR_*`, numbered from 1001 so one telemetry
field can carry either without ambiguity).

## Golden corpus

`golden/index.json` lists each case with its SHA-256. `golden/*.bin` are the
bytes. Both encoders must produce these exact bytes for the described input,
and both readers must accept or reject each one as the index says.

Regenerate with `MIGO_UPDATE_FRAME_WIRE_GOLDEN=1 cargo test -p migo-frame-wire
--features test-support`, and treat a diff as a wire-format change requiring a
version decision — not as a test to update.
