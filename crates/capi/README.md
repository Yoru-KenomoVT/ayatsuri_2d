# ayatsuri-capi

The C ABI of the Ayatsuri2D runtime: `ayatsuri.h`, a shared library and a
static one. Apache-2.0, so a closed game or app can embed it; the editor
and the rest of the workspace stay GPL-3.0.

- [`docs/reference.md`](docs/reference.md): the developer guide and
  reference: quick start, how a frame works, threads, drawing, motions,
  physics, errors, lifetimes, bindings, and every call.
- [`docs/diagrams/`](docs/diagrams/README.md): the diagrams the reference
  calls for, with a brief for each.
- [`docs/replacing-the-sdk.md`](docs/replacing-the-sdk.md): what a host that
  ships the Cubism SDK gets from it, what this ABI covers, and what comes
  next.
- [`include/ayatsuri.h`](include/ayatsuri.h): the header, generated from the
  Rust source by `cargo xtask capi`. Edit the Rust, never the header.
- [`examples/host.c`](examples/host.c): a complete C host, and the
  end-to-end check CI runs against the built library.

Build it alone with `cargo xtask capi` from the workspace root; the result
lands in `target/capi/`. The crate depends on `ayatsuri-core` and
`ayatsuri-format` by path, which is what keeps it inside this repository
for now: splitting it out means publishing those two crates first.
