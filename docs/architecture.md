# Architecture

## The runtime is the product

An editor with nowhere to export to is dead, and a runtime nobody can feed is
dead. But the runtime is what other software embeds, and that is what decides
whether a format lives.

    ayatsuri-format     the puppet format: types, versioning, serialisation
    ayatsuri-core       deformation, physics, masking, animation, draw order
                        no I/O, no renderer, no allocation after load
    ayatsuri-render     renderer behind a trait. Backend not yet chosen.
    ayatsuri-capi       the C ABI, and the header everything else binds to
    ayatsuri-psd        PSD import
    openmoc3            standalone reader for Live2D .moc3 files

## One core, thin bindings

Every binding holds an opaque handle and passes primitives. No binding owns
model state, mirrors it into host objects, or allocates per frame.

This is not an aesthetic preference. The alternative is one framework
reimplemented per language, which is why some existing SDKs are markedly slower
in one language than another: the model lives in the host's objects and the
host's garbage collector decides when you stutter. Here every language is as
fast as the core, because it *is* the core.

The C ABI is therefore not a side deliverable. It is the load-bearing wall, and
it is designed before the bindings rather than after.

## Dependency rules

**`ayatsuri-core` must never depend on `openmoc3`.**

    openmoc3  --parses-->  ayatsuri-format  <--consumes--  ayatsuri-core

Three reasons, in order:

1. `openmoc3` is the hostile-input surface. Isolating it keeps the audit story
   simple and the fuzz target sharp.
2. If moc3 import ever has to be removed, one crate is deleted and the project
   still works.
3. Anyone using only the native format never compiles a moc3 parser.

`openmoc3` is usable on its own, by people who will never adopt the rest of
this project. That is deliberate.

## Build order

    1. ayatsuri-format     everything depends on it
    2. openmoc3            real models are the only large corpus of real rigs;
                           parsing them proves the format can represent what
                           riggers actually make
    3. ayatsuri-core       deformation and physics
    4. ayatsuri-capi       the wall
    5. bindings            prove the wall is coarse enough to be cheap
    6. ayatsuri-render
    7. editor
