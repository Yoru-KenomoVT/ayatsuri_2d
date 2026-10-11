# Ayatsuri2D Open Model format(.AOM) Checklist

- [x] error.rs complete no errors or warnings clean clippy runs

- [x] container.rs complete no errors or warnings clean clippy runs

- [x] buffer.rs complete no errors or warnings clean clippy runs

- [ ] read.rs complete no errors or warnings clean clippy runs

- [ ] write.rs complete no errors or warnings clean clippy runs

- [ ] payload.rs complete no errors or warnings clean clippy runs


# Scope: read and write the `.aom` container

Tier: binary and memory. This is lrbp 23 (walk a hostile binary) plus 26
(design reader and writer together) plus 25 (msgpack), fused into one real
crate.

## Why this one

Everything else in the workspace sits on top of this. `ayatsuri_core` consumes
it, the editor saves through it, and the moc3 bridge maps *into* it. If read
and write drift, every crate above them inherits the drift.

## Where you are

                        done                        empty
                ┌──────────────────┐        ┌──────────────────┐
      error.rs  │ every error the  │        │                  │
                │ crate can raise  │        │                  │
                ├──────────────────┤        ├──────────────────┤
    container.rs│ Header::parse    │        │ read.rs          │  bytes -> Loaded
                │ Header::to_bytes │        │ payload.rs       │  the msgpack schema
                ├──────────────────┤        │ writer.rs        │  Loaded -> bytes
      buffer.rs │ Aligned (16-byte │        │                  │
                │ backing store)   │        │ tests            │  none yet
                └──────────────────┘        └──────────────────┘

`error.rs` already declares `Inflate`, `InflateShort`, `BlobOutRange`,
`BlobMisaligned`, `BlobNotWholeElements`, `Decode`. Those are the contract for
the three empty files. The spec for them is already written, as an enum.

## What it must do

The whole crate is one pipeline and its mirror image:

      READ                                          WRITE

      &[u8] file bytes                              Payload struct
           │                                             │
           ▼ container.rs (done)                         ▼ payload.rs
      Header::parse ──► (Header, &[u8] payload)     serialise msgpack map,
           │                                        pad so each bin lands on 16
           ▼ read.rs                                     │
      flag bit0 set?                                     ▼ writer.rs
       yes: inflate into Aligned, BOUNDED           optionally deflate, set bit0
       no : copy into Aligned                            │
           │                                             ▼ container.rs (done)
           ▼ payload.rs                             Header{len = UNCOMPRESSED len}
      decode msgpack, bins BORROWED                 .to_bytes() ++ payload
           │                                             │
           ▼                                             ▼
      Loaded { header, buffer, payload<'buffer> }   Vec<u8> file bytes

### read.rs

- Take `&[u8]`, call `Header::parse`, branch on `flags.compressed()`.
- Compressed: inflate into `Aligned::zeroed(payload_len)` with the output
  capped at `payload_len`. Fewer bytes produced is `InflateShort`;
  decompressor failure is `Inflate`. Never let the decompressor decide the
  size.
- Uncompressed: copy into `Aligned` (see the alignment trap under Hints for
  why the file slice cannot be borrowed directly).
- Hand the buffer to `payload.rs` and return one owning `Loaded` value. The
  buffer lives as long as the model; nothing allocates after this point.

### payload.rs

- The top-level msgpack map as serde structs. Every section optional, unknown
  keys ignored (that is the entire reason the payload is msgpack per the spec).
- Bulk arrays as `bin`, borrowed from the buffer, not copied. Then a typed
  accessor: `bin bytes -> &[f32]` / `&[u16]` that checks `BlobOutRange`,
  `BlobMisaligned` (offset relative to buffer start is a multiple of 16),
  `BlobNotWholeElements` (len is a multiple of the element size) before
  `bytemuck::cast_slice`.
- Only `meta` and the shape of the sections are in scope now. The spec says
  key names per subsystem are settled as each subsystem lands; do not write
  `physics` or `deformers` today.

### writer.rs

- Serialise a `Payload` to msgpack such that every bin blob's data starts on
  a 16-byte boundary relative to payload byte 0. rmp_serde will not do this
  on its own.
- Optionally deflate; the flag says which. `payload_len` in the header is
  always the uncompressed length.
- Emit `Header::to_bytes()` followed by the payload.

### tests

This is the part lrbp 26 is actually about.

- One rejection test per `Error` variant. Twelve variants, twelve tests, each
  built from a hand-crafted byte array.
- Round trip: `write(p) -> read -> write` is byte-identical, and the decoded
  struct is `==` the original.
- Zero-copy proof: assert a bulk slice's pointer lies inside the buffer's
  range.
- Truncate a valid file at every byte offset and assert `Err`, never a panic.

## What is NOT in scope

      in this crate                    elsewhere
      ─────────────────────────        ──────────────────────────────────
      container header                 moc3 import (openmoc3 -> Payload)
      inflate / deflate                moc3 export and the bundle JSONs
      aligned backing buffer           .aprj / .aodx (they share the
      msgpack shape + meta             header, so they reuse container.rs,
      bin -> typed slice               but their payloads are theirs)
      round trip + rejection tests     textures, file IO, the editor's save UI
                                       per-subsystem schemas (parts, physics...)

Keep `openmoc3` on the far side of the wall. It parses moc3; it must never be
a dependency of this crate.

## Pass criteria

- Round trip is byte-identical, compressed and uncompressed.
- Every `Error` variant has a test that produces it.
- A file truncated at any offset returns `Err`, no panic, no `unwrap` in the
  crate.
- A 570k-float keyform blob decodes as a borrow with zero copies after
  inflate.
- A 12-byte header with `payload_len = 0xFFFFFFFF` is rejected before any
  allocation.
- `cargo clippy` clean, which the checklist above already demands.

## New concepts

- bounded decompression (the small-file, huge-output attack)
- serde borrowing with lifetimes (`Payload<'buf>`)
- alignment as a *format* invariant, not just a memory one
- pointer-offset validation before a `bytemuck` cast

## Build order

1. `read.rs` uncompressed path only: header, copy into `Aligned`, return it.
   Feed it a hand-made 12-byte header plus a few bytes.
2. Add the inflate branch with the bound. Test `InflateShort` by declaring a
   length larger than the stream produces.
3. `payload.rs` with just `meta` and one bin field. Get a borrowed slice out
   and print its pointer next to the buffer's pointer.
4. Add the typed accessor and the three blob rejection tests.
5. `writer.rs` uncompressed, no bins yet. Round-trip `meta`.
6. Add the bin padding. Round-trip a bin and assert the offset is `% 16 == 0`.
7. Add deflate. Round-trip both ways.
8. The truncation sweep. Remove every `unwrap`. Clippy.

## Hints, shape only

**The 12-byte header is not 16-aligned.** The spec says blobs are aligned
relative to *payload start*, and the payload starts at file offset 12. So even
for an uncompressed file read straight into a `Vec<u8>`, the payload begins 12
bytes into the allocation and a blob at payload offset 16 sits at file offset
28. The "no copy for uncompressed" promise in the spec's reading step 3 cannot
hold as written. Three ways out, and it is a decision to make before writing
`read.rs`, because `writer.rs` must agree:

      a) copy uncompressed into Aligned too       simplest, one copy per load
      b) pad the header to 16 bytes               CONTAINER_VERSION bump, spec edit
      c) align blobs relative to FILE start       reader math changes, mmap works

**Padding has to account for the bin header.** A msgpack bin is `0xC4 len:u8`,
`0xC5 len:u16`, or `0xC6 len:u32` followed by data. The *data* must land on
16, so the padding goes before the marker and its size depends on which marker
is emitted. Draw the byte positions for a 300-byte blob before writing code.

**rmp_serde does not know what alignment is.** Either drive `rmp::encode` by
hand for the sections that hold bins, or find a trick that keeps serde in
charge (a reader that ignores unknown keys will happily skip a `"_pad": bin`
key, for example). Both work; one is more code, the other puts padding in the
schema. Weigh them.

**Three things in the finished files worth a second look before building on
them:**

- `docs/aom-format.md` lists payload length at `0x0A`, but `container.rs`
  packs it at `0x08` (4+2+2 = 8). The doc has a typo, the code is right. Fix
  the doc so the next reader of the spec does not "correct" the code.
- `container.rs` returns `TooNew(version)` for unknown *flag* bits. That is a
  different failure and the message will mislead whoever hits it.
- `buffer.rs` `as_slice` takes `&mut self`. That means two read-only views of
  the buffer cannot coexist, which is exactly what `Payload<'buf>` needs.

## Neighbours

Previous: `container.rs` and `buffer.rs` (done).
Next: whichever crate first calls `read()`, probably `ayatsuri_core`.
