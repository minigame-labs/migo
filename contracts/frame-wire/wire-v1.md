# WireFramePacket v1

The only specification of the bytes that cross a process boundary between a
Migo frame producer and a Migo renderer. Two implementations exist — a
JavaScript encoder running inside WebKit's WebContent process, and the Rust
reader in `engine/crates/frame-wire` — and neither is the specification.

Both are checked against the fixed corpus in `golden/`. Checking them against
each other instead would let a shared misreading pass: two encoders that agree
with one another and not with a fixed corpus is exactly the failure a corpus
exists to catch.

## Status: refrozen, and why that was allowed

This document describes v1 after a correcting pass (2026-09-03) that changed
the header layout, retired a flag, and tightened the section rules. Rewriting a
shipped format in place would be indefensible; an audit established that this
one had not shipped:

- `contracts/frame-wire/` and `engine/crates/frame-wire/` exist only on the
  `feat/apple-platform-foundation` branch. `master` does not contain them, and
  neither does any tag — including `v0.9.6`, the current release.
- No `migo_session_submit_external_frame` entry point is exported anywhere, so
  no host can have sent a packet.
- The only in-repository consumer of the crate is the command-stream validator
  (`gl_stream`), which does not read the envelope.

So there is no reader in the field to break. **This is the last renumber.** The
next change to a field offset, a flag bit, or a rejection code is a
`wire_version` bump with both versions accepted for a transition, because by
then a shipped Swift transport and a shipped JavaScript encoder will disagree
with anything else.

## Conventions

- Little-endian. Every multi-byte field.
- Offsets are byte offsets from the first byte of the packet.
- **No alignment may be assumed of the packet's own base address.** It arrives
  on the reading side inside `Data.withUnsafeBytes`, which promises nothing, so
  a reader must not cast a pointer to a multi-byte type. Section *payloads* are
  8-byte aligned relative to the packet start, which is the guarantee a reader
  can use once it has copied or confirmed the base alignment itself.

## Header — 80 bytes, fixed

| Offset | Size | Field | Rule |
|---:|---:|---|---|
| 0 | 4 | `magic` | exactly `0x4D475046` |
| 4 | 4 | `wire_version` | exactly `1` for this document |
| 8 | 4 | `header_bytes` | exactly `80`. Validated, not trusted: a header that could announce its own length would let a producer move the section table |
| 12 | 4 | `total_bytes` | `>= 80`, `<= 4194304`, and **equal to the delivered byte count** |
| 16 | 16 | `launch_nonce` | 128-bit, unguessable, generated once per app launch and paired with one ingress. A packet with any other value is not this producer's |
| 32 | 8 | `sequence` | **strictly contiguous** per runtime generation: the next accepted value is exactly the previous plus one, and the first is `1` |
| 40 | 8 | `runtime_generation` | the producer generation these bytes were built by |
| 48 | 8 | `surface_generation` | the surface they were built against; `0` means "not surface-bound" |
| 56 | 8 | `resource_epoch` | the resource-table epoch the ids inside are valid in |
| 64 | 4 | `frame_id` | producer's own frame counter; advisory, for latency attribution. Checksummed like everything else, but nothing is derived from it |
| 68 | 4 | `flags` | exactly `PRESENT` in v1; see below |
| 72 | 4 | `section_count` | `<= 8`. Zero is rejected too, by `MissingCommandStream`, which says why rather than counting |
| 76 | 4 | `payload_checksum` | CRC32 of the whole packet with these four bytes read as zero |

There is no reserved word. The header is 80 bytes with every byte meaningful,
and 80 is a multiple of 16, so the 16-byte section table that follows is
naturally aligned and the first section payload needs no gap. A reserved field
would buy nothing a `wire_version` bump does not already buy: a new *required*
field is a version change whether or not space was set aside for it.

### Widths, and why they are not smaller

`launch_nonce` is 128-bit because it is the value that decides whether bytes
arriving from another process belong to this producer. 64 bits is enough against
accidental collision and is not the standard for an unguessable identifier;
this field is also the shape the transport's per-generation bearer token is
derived alongside, and both are sized for the guessing case rather than the
accident case. (The bearer token itself is **not** in this header. It
authenticates the connection, not the packet, and putting it in every frame
would put it in every buffer, log and crash dump that ever holds a frame.)

`sequence`, `runtime_generation`, `surface_generation` and `resource_epoch` are
64-bit. A 32-bit generation that wraps is a generation that eventually matches a
stale packet exactly, which is the one failure the field exists to prevent, and
at 60 Hz a 32-bit sequence wraps in under two years of continuous running.
Mixing widths inside one header to save eight bytes on a packet whose payload is
measured in kilobytes would be a false economy.

### Flags

| Bit | Name | Meaning |
|---:|---|---|
| 0 | `PRESENT` | this packet is a complete frame: execute it and end the frame |

`PRESENT` is **required**. Any other bit set is a rejection, and a packet
without it is a rejection.

**v1 has no `CONTINUED` flag, and no semantic frame continuation.** An earlier
draft had one. A packet that carries drawing work but does not end a frame is a
packet whose effects a *later* packet depends on, which means a rejected or
lost middle packet leaves the renderer holding half a frame — and makes every
question about credits, sequence gaps and generation loss a question about
partial state. With a 4 MiB ceiling and real frames measured in tens of
kilobytes, continuation buys nothing and costs that entire class of bug.
Requiring `PRESENT` is how the absence is enforced rather than merely intended.

Transport-level fragmentation is a different thing and is still allowed: a
transport that splits bytes reassembles them **before** the parser is called,
under the same ceiling. The parser contains no reassembly code, and
`total_bytes` must equal the delivered length, so a fragment cannot be mistaken
for a packet.

## Section table

`section_count` entries of 16 bytes each, starting at offset 80.

| Offset | Size | Field |
|---:|---:|---|
| 0 | 4 | `kind` |
| 4 | 4 | `offset` |
| 8 | 4 | `byte_length` |
| 12 | 4 | `item_count` |

### Canonical layout

The packet has exactly one legal byte layout for a given list of sections:

1. `section[0].offset == 80 + 16 * section_count`.
2. `section[i].offset == align_up_8(section[i-1].offset + section[i-1].byte_length)`.
3. `offset + byte_length <= total_bytes`, computed with checked arithmetic. The
   unchecked form wraps, and a wrapped end compares as "fits".
4. `total_bytes == align_up_8(last.offset + last.byte_length)` exactly — neither
   trailing bytes past it nor a missing final pad.
5. Every padding byte — the 0 to 7 bytes between a section's end and the next
   section's start, and the same after the last one — is zero.
6. No kind appears twice.

Rules 1, 2 and 4 make ascending order, disjointness and 8-byte alignment
consequences rather than separate checks: there is no offset that satisfies them
and also overlaps, sits out of order, or lands unaligned. Rules 4 and 5 close
the rest: there is no byte in a valid packet that is not header, table, declared
section payload, or a zero pad.

That last property is the point of calling the layout canonical, and it is worth
more than tidiness. A format that tolerates gaps has room in it — room the
checksum covers and no consumer interprets, which is where a second channel
hides. It also makes the golden corpus mean something: byte-for-byte agreement
between two independent encoders is only a real check if one input has one
encoding.

### Per-kind `item_count`

| Value | Name | Required | `item_count` rule | Contents |
|---:|---|---|---|---|
| 1 | `COMMAND_STREAM` | yes | `byte_length % 4 == 0` and `item_count <= byte_length / 4` | `u32` words: the drawing commands |
| 2 | `INLINE_DATA` | no | `item_count <= byte_length` | small blobs referenced by the command stream |
| 3 | `RESOURCE_REFERENCES` | no | `byte_length == item_count * 4` | one `u32` resource id per reference |
| `0x80000001` | `DAMAGE` | no | `byte_length == item_count * 16` | advisory damage rectangles, four `u32` each |
| `0x80000002` | `TIMING` | no | `item_count <= byte_length` | advisory producer timestamps |

Records are a fixed width wherever one exists, and then `item_count` is pinned
to `byte_length` exactly rather than bounded by it. A count that merely fits is
a count the consumer will loop on and a length the consumer will trust, and the
two disagreeing is how a reader walks off the end of the meaningful data while
staying inside the buffer. `COMMAND_STREAM` records are variable length by
construction, so the bound is all the envelope can say; `INLINE_DATA` and
`TIMING` carry shapes the envelope deliberately does not know.

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

## Ceilings

| Ceiling | Value | Where it lives |
|---|---:|---|
| Absolute packet size | 4 MiB | `MAX_TOTAL_BYTES`, a compile-time constant in the parser |
| Per-session packet size | 4 MiB by default, **lowerable only** | `FrameIngress::with_max_packet_bytes` |
| Sections per packet | 8 | `MAX_SECTIONS` |
| Credits in flight | 2, **compile-time maximum** | `MAX_CREDITS`; `with_max_credits` clamps into `1..=MAX_CREDITS` |

Two size ceilings, not one, because they answer different questions.
`validate` is a pure function with no session in scope, so it needs a constant;
a host that knows its device is tight needs to say something smaller. Both are
enforced, and neither can be raised at runtime: `with_max_packet_bytes` and
`with_max_credits` clamp instead of trusting, so a value arriving from
configuration, a remote policy, or a content manifest can only tighten the
parser, never loosen it. Raising a ceiling is a code change with device
measurements behind it.

4 MiB is the whole packet including the envelope, and it is also the largest
payload class in the G0 probe matrix. A heavy real frame measures in tens of
kilobytes; the ceiling exists so that a bogus `total_bytes` is rejected rather
than believed, not to accommodate an expected size.

Two credits, so the producer can build frame N+1 while the renderer works on N,
and no deeper: every additional credit is another frame of input latency and
another packet's worth of memory in flight.

## Identity, ordering and resource admission

These are the ingress's rules rather than the parser's — the parser cannot know
them, because they depend on state the host owns.

- **Identity.** `launch_nonce` and `runtime_generation` must match the ingress
  exactly. A wrong nonce is a foreign packet. A wrong runtime generation is
  reported as *generation lost* rather than as an error: the WebContent process
  was replaced or the session reloaded, nobody did anything wrong, and no retry
  helps.
- **Sequence.** Strictly contiguous. A repeat is a replay, a gap means a packet
  carrying state was lost, and a decrease is a reorder; each would draw a frame
  twice, incompletely, or out of order. Contiguity is affordable precisely
  because a rejection is not recoverable in the first place:
  `contracts/apple/profile-policy.json` answers `wire_validation_failed` by
  terminating the content and voiding the generation, so there is no "skip the
  bad one and continue" path for a gap to serve.
- **Timeline.** `surface_generation` and `resource_epoch` only ever advance. The
  host's setters refuse to move either backwards and say so, rather than
  quietly accepting a value that would make a stale packet valid again.
- **Resource admission.** A packet carrying `RESOURCE_REFERENCES` is rejected
  until the host has declared the current epoch's resources ready. Advancing the
  epoch clears that state, because an epoch advance means the resource table was
  rebuilt and nothing in it is ready yet by definition. In v1 readiness is
  per-epoch; the per-resource, hash-verified form arrives with the resource
  protocol and can only narrow this rule.
- **Validity does not depend on load.** Every check above runs before the credit
  check, so whether a packet is *legal* never depends on how busy the renderer
  is. The alternative answers "wait" to malformed bytes and invites the producer
  to resend them forever.

## Checksum

CRC32 (IEEE) over the entire packet, with `payload_checksum`'s own four bytes
substituted with zero.

Covering the header — not only the payload — is what catches the corruption
that still parses. A flipped `frame_id` or `section_count` leaves a
structurally perfect packet; nothing but a checksum over the header notices.

The checksum is the **last** envelope check to run. It is the only pass over
the whole packet, and a malformed header should never cost a full CRC.

## Rejection codes

Stable, and carried across the C ABI into host telemetry. The list lives in
`engine/crates/frame-wire/src/lib.rs` (`WireError`, from 1) and
`src/ingress.rs` (`INGRESS_ERROR_*`, from 1001 so one telemetry field carries
either without ambiguity).

| Code | Name | Meaning |
|---:|---|---|
| 1 | `ShorterThanHeader` | fewer bytes than the fixed header |
| 2 | `BadMagic` | magic is not `MGPF` |
| 3 | `UnsupportedVersion` | `wire_version` is not 1 |
| 4 | `BadHeaderBytes` | `header_bytes` is not 80 |
| 5 | `BadTotalBytes` | `total_bytes` is below the header or above the absolute ceiling |
| 6 | `LengthMismatch` | `total_bytes` does not equal the delivered byte count |
| 7 | `TooManySections` | `section_count` is above the cap |
| 8 | `SectionTableOutOfBounds` | the section table does not fit in the packet |
| 9 | `SectionOutOfBounds` | a section extends past the end of the packet |
| 10 | `SectionNotCanonical` | a section does not start where the canonical layout puts it — which is also how misalignment, overlap and disorder present |
| 11 | `PaddingNotZero` | an alignment pad byte is not zero |
| 12 | `DuplicateSection` | the same section kind appears twice |
| 13 | `UnknownRequiredSection` | an unknown non-advisory section kind is present |
| 14 | `ChecksumMismatch` | the payload checksum does not match |
| 15 | `UnknownFlags` | a flag bit outside v1 is set |
| 16 | `MissingPresent` | `PRESENT` is not set |
| 17 | `CommandStreamNotWordAligned` | the command stream is not a whole number of words |
| 18 | `ItemCountInconsistent` | `item_count` disagrees with `byte_length` for this kind |
| 19 | `MissingCommandStream` | the packet carries no command stream |
| 20 | `TotalBytesNotCanonical` | `total_bytes` is not the 8-byte-aligned end of the last section |
| 1001 | `FOREIGN_SESSION` | `launch_nonce` is not this ingress's |
| 1002 | `NONCONTIGUOUS_SEQUENCE` | `sequence` is not exactly the previous plus one |
| 1003 | `STALE_SURFACE` | built against a retired surface generation |
| 1004 | `STALE_RESOURCE_EPOCH` | names ids from another resource epoch |
| 1005 | `PACKET_TOO_LARGE` | above this session's packet ceiling |
| 1006 | `RESOURCES_NOT_READY` | carries resource references before the epoch's resources were declared ready |

Codes 10 and 18 each cover what earlier drafts split into two. The finer codes
were not lost by accident: with a canonical layout there is no input that is
misaligned but canonical, or overlapping but canonical, so the extra codes were
branches that could not be reached. An unreachable diagnostic is worse than one
message that names all three cases, because the next reader assumes it fires.

## Golden corpus

`golden/index.json` lists each case with its SHA-256 and whether a reader must
accept it. `golden/*.bin` are the bytes. Both encoders must produce these exact
bytes for the described input, and both readers must accept or reject each one
as the index says — including the rejected case, whose `wire_error` the index
names. A corpus with nothing rejected in it never exercises the half of the
reader that says no.

Regenerate with `MIGO_UPDATE_FRAME_WIRE_GOLDEN=1 cargo test -p migo-frame-wire
--features test-support`, and treat a diff as a wire-format change requiring a
version decision — not as a test to update.
