# Ayatsuri2D SDK {version} ({label})

The Ayatsuri2D runtime as a C library: load `.aom` and Cubism `.moc3`
models, set their parameters, update (motions, expressions and physics),
and draw the meshes it hands back with whatever you draw with.

## What is inside

- `include/ayatsuri.h`: the one header.
{libs}
- `examples/host.c`: a complete C host that loads a model, sets parameters,
  plays a motion, runs physics and walks the drawables.
- `LICENSE`, `LICENSE-MIT` and `THIRD-PARTY.md`: the licences, below.

## Linking

Shared, loaded as a plugin or beside your program:

    cc host.c -Iinclude -Llib -layatsuri_capi

Static, compiled into your program; your linker keeps only the calls you
use:

    cc host.c -Iinclude lib/libayatsuri_capi.a -Wl,--gc-sections -lpthread -ldl -lm

On macOS use `-Wl,-dead_strip -framework CoreFoundation` instead of the
last three flags. On Windows with MSVC, link `ayatsuri_capi.dll.lib` and
ship `ayatsuri_capi.dll` beside your program, or link `ayatsuri_capi.lib`
statically.

Check `ay2d_abi_version()` against `AY2D_ABI_VERSION` once at startup.

## Documentation

The guide and every call: https://dev.ayatsuri-2d.org/

## Licences

The C ABI is Apache-2.0 (`LICENSE`). The runtime it is built on
(`ayatsuri-core`, `ayatsuri-format`, `ayatsuri-import`) is MIT
(`LICENSE-MIT`). The Rust crates linked into the library keep their own
licences, all reproduced in `THIRD-PARTY.md`. A closed-source game or app
may ship this library; keep these files with it.
