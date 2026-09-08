# The format

Models are saved as **`.aom`**, the Ayatsuri Open Model: the Alterion container
shared with `.aodx` and `.aprj`, carrying a MessagePack payload with bulk arrays
stored as raw `bin` blobs. Self-describing where that buys tolerant versioning,
flat where that buys zero-copy. The byte layout is specified in
[aom-format.md](aom-format.md).

## Ours is a superset

    openmoc3 --parses--> ayatsuri format <--consumes-- ayatsuri-core

moc3 import maps *into* our format; it does not define it. The relationship is
the one Blender has with FBX, or Krita with PSD: the native format is richer,
the interchange format is a lossy door.

    everything moc3 can express      round-trips back to moc3
    everything we add on top         does not

The constraint bites in exactly one place: exporting back to moc3 so a model
runs in existing applications today. That is a per-model choice, not a ceiling
on the project.

## Two delivery paths, and moc3 export is not optional

A puppet has to reach people, and today that means the existing ecosystem.

    host                              loads                    features
    ------------------------------    ---------------------    -----------------
    VTube Studio, Unity, Unreal,      an exported moc3          capped at what
    Godot, anything on the Cubism     BUNDLE                    moc3 can express
    SDK

    any host linking ayatsuri-capi    .aom natively,            everything
                                      and moc3 as well

So **every `.aom` must be exportable as a moc3 bundle.** That is a hard
requirement, not a convenience: a model nobody can load is not a model. Native
`.aom` is the better path wherever the host supports it, and the fallback is
always there.

### What a bundle actually contains

Not just the binary. A runnable Live2D model is a directory:

    model.moc3              the binary.        <- the only hard part
    model.model3.json       manifest           <- published format
    model.physics3.json     physics rig        <- published format
    model.cdi3.json         display names      <- published format
    model.pose3.json        part groups        <- published format
    *.exp3.json             expressions        <- published format
    motions/*.motion3.json  animation          <- published format
    textures/*.png          atlases

Every JSON file there is **publicly specified** by Live2D, so writing them
needs no reverse engineering at all. The `.moc3` binary is the only piece that
does, which means the bundle exporter is mostly straightforward work sitting on
top of one hard component.

### Consequence for the schedule

The moc3 **writer** is therefore a required deliverable, not an optional extra.
It is currently blocked on a small number of sections whose trailing bytes are
not zero, meaning their element size or count is still slightly wrong in our
map. Reading is unaffected; writing cannot be correct until those are resolved.

## Compatibility is chosen at project creation

A rigger who builds a model with hair collision and custom blend modes, then
discovers at export that none of it survives, has lost a day.

So the target is chosen when the project is created, not when it is exported:

    target: moc3 compatible    the editor constrains the palette from the start
    target: native             everything available, needs an Ayatsuri2D host

A lint that only fires at the end is the wrong ergonomics. A game engine picks
its target platform first, and so does this.

## Version targeting on export

moc3 has versions, and older hosts **refuse** newer files outright rather than
degrading. So the exporter emits the *lowest version that can express the
model*, automatically, and the editor says so at the moment a rigger picks a
feature that forces the version up.

## What we add

    physics with collision       hair that does not pass through the body
    full blend mode set
    nested masking without an arbitrary cap
    2D bone skeletons alongside mesh deformation
    vector / SDF art, resolution independent
    per-drawable effect stack
    colour, gradient and curve parameters

Hair clipping through the body is the single most complained-about limitation
in existing tools. Riggers fake it with hand-authored keyforms, hours per model,
because the pendulum physics used elsewhere has no notion of anything to collide
with. That is a feature people would switch tools for, which is a much higher
bar than a feature people would like.

## Migration

PSD is the path, not moc3 import. A `.psd` predates any runtime format, so
re-rigging from source art is clean, and it produces a native rig with the
features above rather than a lossy copy of what the rigger already had.
