# .aom : the Ayatsuri Open Model format

Normative. This is what a reader must accept and a writer must produce.

## Container

The Alterion container, shared with `.aodx` and `.aprj`, so header handling is
one implementation across every project in the family.

    offset  size  field
    ------  ----  ----------------------------------------------------------
    0x00       4  magic, ASCII "AOMX"
    0x04       2  container version, u16 little endian
    0x06       2  flags, u16 little endian
    0x0A       4  uncompressed payload length, u32 little endian
    0x0C          payload begins

    flags bit 0   payload is deflate compressed
    bits 1-15     reserved, must be zero

**Container version is bumped only when this 12-byte header changes**, never
when the model gains a field. MessagePack tolerates added fields on its own,
which is the entire reason the payload is MessagePack.

A reader **must** reject a declared payload length above a sane ceiling before
allocating anything (256 MB is the value used elsewhere in the family), and
**must** bound the decompressor so a small file cannot inflate without limit.

## Payload

A MessagePack map. Keys are strings, so the format is self-describing and a
reader skips what it does not recognise.

    {
      "meta":       { name, author, canvas, created, tool }
      "parts":      { ids[], parent[], visible[], enabled[] }
      "deformers":  { ids[], kind[], parent[], grid... }
      "meshes":     { ids[], texture[], flags[], parent[], spans... }
      "params":     { ids[], min[], max[], default[], kind[] }
      "keyforms":   { bindings..., positions: bin }
      "geometry":   { vertices: bin, uvs: bin, indices: bin }
      "physics":    { chains[], colliders[], solver[] }
      "masks":      { ... }
      "draworder":  { ... }
      "presets":    { ... }
    }

Exact key names are settled per subsystem as each is implemented. Two rules
hold regardless.

### Rule 1: bulk arrays are `bin`, never lists

Anything with more than a few hundred elements, so vertices, UVs, indices,
keyform positions and colour keyforms, is stored as a MessagePack **`bin`**
blob containing the values packed little endian, exactly as a flat array.

This is not a micro-optimisation. Measured on a real 570,608-float keyform
array:

    flat f32 array                  2,282,432 bytes
    MessagePack, floats as a LIST   5,135,477 bytes   2.25x larger, 27.8ms decode
    MessagePack, floats as a BIN    2,282,441 bytes   1.00x,          0.1ms decode

A `bin` blob costs nine bytes over raw and decodes as a borrow. A list of
numbers costs 2.25x the space and tags every single value.

### Rule 2: blobs are 16-byte aligned

Each `bin` blob **must** begin on a 16-byte boundary relative to the start of
the payload, padded with zeros if necessary.

A reader that inflates into an aligned buffer can then cast a blob directly to
`&[f32]` or `&[u16]` with no copy. Without the guarantee, every bulk array
needs either unaligned reads or a copy, which is the cost the `bin` encoding
exists to avoid.

## Reading

    1. read 12 bytes; magic must be "AOMX"
    2. reject an implausible declared length before allocating
    3. if flags bit 0: inflate into ONE aligned buffer, bounded
       else: the payload is the buffer
    4. parse the MessagePack structure from that buffer
    5. bulk arrays are SLICES INTO the buffer, borrowed, never copied

**The buffer is the backing store for the model's lifetime.** Every vertex
array and keyform block points into it. After load, nothing allocates.

## Compression policy

Optional, and the flag says which. Compress when saving from the editor,
because models get shared and downloaded and it roughly halves them. Leave it
uncompressed when producing a build for an embedder that wants to memory-map,
on a console or an embedded target where a load-time allocation is unwelcome.

Same format, same reader, one bit different. Decompression is linear in output
size and costs roughly 4ms per megabyte, so a typical model pays 3-15ms once
per load.

## Relationship to moc3

`.aom` is a superset. Everything moc3 can express maps into it and round-trips
back out; what it adds does not. See [format.md](format.md).
