# Provenance

Ayatsuri2D is an independent implementation. It contains no Live2D source code.

- No Live2D binary has been disassembled or decompiled by this project.
- No Live2D SDK or framework source has been read or ported.
- No contributor has accepted the Live2D Cubism SDK or Editor licence
  agreements.

Format compatibility derives from three sources:

1. **Live2D's own published specifications**, covering the JSON sidecar
   formats, and the published standard parameter list.
2. **Permissively licensed community work**: the OpenL2D pattern definition,
   whose licence explicitly permits building readers and writers from it, and
   Sakura Motion's PurismCore, which is MIT.
3. **Independent analysis of model files** obtained under their creators'
   terms. No third-party model is redistributed by this project. Method and
   findings are kept as a dated record.

## Why this is stated at all

Two widely used implementations of this format derive from a decompilation of
the vendor's own exporter. They are more complete than the clean alternatives
in places, which is exactly the problem: anything built on them inherits that
lineage, and cannot be vendored by anyone with a legal department.

Staying clean is not a moral posture. It is what allows this to be embedded,
which is the entire point of building a runtime.

---

Live2D and Cubism are trademarks of Live2D Inc. This project is not affiliated
with, endorsed by, or supported by Live2D Inc. Please do not contact them about
it.
