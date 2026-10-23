<div align="center">
    <picture>
        <source media="(prefers-color-scheme: dark)" srcset="assets/logo-dark.png">
        <source media="(prefers-color-scheme: light)" srcset="assets/logo-light.png">
        <img alt="Ayatsuri2D" src="assets/logo-light.png" width="380">
    </picture>

# 操り2D · Ayatsuri2D

**A free, open-source editor for rigging and animating 2D puppets.**<br>
Bring in your layered artwork, rig it, give it physics, and export a model that
runs in VTube Studio. On Windows, macOS and Linux.

[![Latest release](https://img.shields.io/github/v/release/Yoru-KenomoVT/ayatsuri_2d?include_prereleases&label=download&color=e85d8a)](https://github.com/Yoru-KenomoVT/ayatsuri_2d/releases)
[![CI](https://github.com/Yoru-KenomoVT/ayatsuri_2d/actions/workflows/ci.yml/badge.svg)](https://github.com/Yoru-KenomoVT/ayatsuri_2d/actions/workflows/ci.yml)
[![License: GPL-3.0](https://img.shields.io/badge/license-GPL--3.0-blue)](LICENSING.md)
![Platforms](https://img.shields.io/badge/Windows%20·%20macOS%20·%20Linux-555)
![moc3](https://img.shields.io/badge/moc3-3.0%20to%205.3-brightgreen)
[![Rust](https://img.shields.io/badge/Rust-2024-orange?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Website](https://img.shields.io/badge/ayatsuri--2d.org-website-f08e1e)](https://ayatsuri-2d.org)
[![Discord](https://img.shields.io/badge/Discord-join-5865F2?logo=discord&logoColor=white)](https://discord.gg/WyzDsXzdnh)

[**Website**](https://ayatsuri-2d.org) · [**Discord**](https://discord.gg/WyzDsXzdnh) · [**Download**](#download) · [Features](#features) · [Status](#status) · [Build from source](#build-from-source) · [Docs](#documentation) · [Support the project](#support-the-project)

</div>

<p align="center">
  <img src="docs/media/hero.png" alt="The Ayatsuri2D editor with a rigged model: the face mesh, parameter folders with 2D pads, the deformer tree and parts" width="900">
  <br>A rigged model in the editor: mesh wireframe, keyed parameters in folders, the deformer tree and parts.
  <br>Model art by <a href="https://www.pixiv.net/users/99163365"><b>マパ*MAPA</b></a> (pixiv ID 99163365), used with permission. Thank you!
</p>

---

`操り` (*ayatsuri*) is the working of a puppet: the strings, the hands, the
craft of making a jointed figure move as though it were alive. That is what
this software does, and what a rigger does.

## What is it?

Ayatsuri2D is a rigging editor for **Live2D-style models**, the moving 2D
characters used by VTubers, games and visual novels. You start from a layered
drawing, cut each layer into a mesh, and connect the meshes to **parameters**
(head angle, eye open, mouth shape) through deformers. Move a slider and the
drawing turns its head, blinks, and its hair swings.

It reads and writes the formats the rest of the ecosystem already uses:

- **Opens** Cubism models (`.model3.json` and `.moc3`, versions 3.0 to 5.3),
  Cubism Editor projects (`.cmo3`, rig and all), and Photoshop `.psd`,
  Krita `.kra` and Clip Studio Paint `.clip` files.
- **Exports** a complete moc3 bundle (model, physics, motions, expressions,
  textures) that loads in VTube Studio, OpenVT and anything built on the
  Cubism SDK, and a Cubism Editor project (`.cmo3`) with the rig and the
  art as layers.
- **Saves** its own `.aom` (Ayatsuri Open Model) format, a documented, open
  superset of moc3.
- **Talks** Cubism Editor's integration API, so tools made for Cubism Editor
  (like VTube Studio's parameter sync) connect to it unchanged.

It is an independent, clean-room project, not a clone and not affiliated with
Live2D Inc. See [Provenance](#provenance).

## Download

[![Latest version](https://img.shields.io/github/v/release/Yoru-KenomoVT/ayatsuri_2d?include_prereleases&label=latest&style=for-the-badge&color=e85d8a)](https://github.com/Yoru-KenomoVT/ayatsuri_2d/releases)
[![Release date](https://img.shields.io/github/release-date-pre/Yoru-KenomoVT/ayatsuri_2d?label=released&style=for-the-badge)](https://github.com/Yoru-KenomoVT/ayatsuri_2d/releases)

Grab the latest build from the
[**releases page**](https://github.com/Yoru-KenomoVT/ayatsuri_2d/releases),
or pick your system on [**ayatsuri-2d.org/download**](https://ayatsuri-2d.org/download/),
which has first-launch notes for each one.
These badges always show the newest version, betas included.

| System | File | Notes |
|---|---|---|
| **Windows** 10 / 11 | `…-windows-x64-setup.exe` | Installer with Start Menu entry and `.aom` file association |
| | `…-windows-x64.zip` | Portable: unzip and run `ayatsuri-launcher.exe` |
| **macOS** 11+ | `…-macos-universal.dmg` | Apple Silicon and Intel |
| **Linux** | `…-linux-x86_64.AppImage` | Any distro: mark it executable and run |
| | `.deb` / `.rpm` | Debian, Ubuntu, Mint / Fedora, openSUSE |
| | `.tar.gz` | Unpack and run `./install.sh` (per user, no root) |

> [!NOTE]
> The builds are not signed by Apple or Microsoft yet. On **macOS**, the first
> time, right-click the app and choose **Open**. On **Windows**, SmartScreen may
> warn: choose **More info**, then **Run anyway**. Every release lists SHA-256
> checksums in `SHA256SUMS.txt`.

### System requirements

| | Minimum | Recommended |
|---|---|---|
| **OS** | Windows 10 64-bit, macOS 11, or Linux x86_64 (glibc 2.35+) | same |
| **CPU** | 2 cores, 64-bit | 4 cores or more |
| **Memory** | 1 GB free | 2 GB or more for very large models |
| **Graphics** | integrated graphics with OpenGL 3.3, Vulkan, DirectX 12 or Metal | any dedicated GPU |
| **Disk** | 100 MB for the app, plus your models | more if you record timelapses, which grow with a model's history |
| **Screen** | 1280×720 | 1920×1080 |
| **Video export** | none for GIF and PNG frames | [ffmpeg](https://ffmpeg.org) for MP4, WebM and MOV: the editor and installers download it for you, no PATH setup |

Measured, not guessed: a full VTuber model (356 meshes, 19,405 points,
28,385 triangles, 229 deformers, 114 parameters) on **2 CPU cores and
integrated graphics** opens in 1.6 seconds, uses 486 MB of memory and about
480 MB of video memory, and evaluates in under 5 ms a frame.

## Features

### From artwork to mesh

<p align="center">
  <img src="docs/media/mesh-edit.png" alt="Mesh Edit on a face line: the mesh over the art, with the Edit Mesh popup beside it (manual tools, Mirror Edit and the Automatic mesh presets)" width="860">
</p>
<p align="center">Model art by <a href="https://www.pixiv.net/users/99163365"><b>マパ*MAPA</b></a> (pixiv ID 99163365), used with permission. Thank you!</p>

- **PSD, Krita and Clip Studio import**: folders become parts, layers become
  meshes packed into texture atlases, clipping layers become masks. Import
  into a new model, or add more art to a model you already have.
- **Cubism project import and export**: a `.cmo3` opens with its meshes,
  deformer tree, parameters and folders, keyforms, clipping, physics and
  names, and a model exports as one, art and all, for riggers who finish in
  Cubism Editor.
- **Mesh editing**: auto mesh that follows the outline of the art (Standard,
  Deform and Light presets), add and erase points, connect and cut edges, an
  erase brush, 4-division. Points slide over the art instead of stretching it.
- **Lasso, copy and paste points** between meshes, merging with the points
  already there so two meshes line up.
- **Glue** to stitch overlapping meshes together, with a weight brush.
- **Clipping masks**, picked straight on the canvas.

### Rigging

<p align="center">
  <img src="docs/media/rigging.png" alt="The Head Main warp deformer with its Bezier handles, keyed to the head parameters, over the whole model" width="860">
</p>
<p align="center">Model art by <a href="https://www.pixiv.net/users/99163365"><b>マパ*MAPA</b></a> (pixiv ID 99163365), used with permission. Thank you!</p>

- **Parameters and keyforms**, with Cubism's palette tools: four-corner forms,
  motion mirroring, Reflect, Adjust, Change, parameter folders and linked pads.
- **Warp deformers** (grid points, the transform box, Bezier handles) and
  **rotation deformers**. Re-parent by drag and drop; nothing on screen moves.
- **Live, exact dragging**: the art follows every drag as it happens, and a
  point lands exactly where you let go, even through bent warps.
- **Deform paths**, **brush selection** with soft weights, keyed **opacity**
  for meshes and deformers, blend modes, draw order.

### Physics and animation

<p align="center">
  <img src="docs/media/physics.png" alt="The Physics page mid-swing: input and output settings, the pendulum preview and the model blinking" width="860">
</p>
<p align="center">
  <img src="docs/media/animation.png" alt="The Animation workspace: an animation's keys on the Timeline, its parameters in folders and the posed model" width="860">
</p>

- **Physics page** laid out like Cubism's, with a live pendulum preview that
  follows the real parameters. Reads and writes `physics3.json`.
- **Animation scenes** with curves: auto smooth, linear, stepped, inverse
  stepped and free Bezier keys, events and fades. Reads and writes
  `motion3.json` key for key.
- **Compositions and sound**: animations and other compositions layered on
  one clock (start, trim, speed, weight, fades), with WAV, MP3, FLAC or Ogg
  sound layers and an **Audio Mixer** with a bus per audio source. Baked to
  one `motion3.json` on export.
- **Expressions**: reads and writes `exp3.json` (Add, Multiply, Overwrite).
- **Render**: animations, compositions and timelapses to GIF, PNG frames,
  MP4, WebM or MOV, drawn on the GPU, with the composition's sound mixed in.

### Built to work with what you already use

- **moc3 export** in versions 3.0, 3.3, 4.0, 4.2, 5.0 and 5.3, checked against
  real Cubism exports. 5.0 is the default because VTube Studio does not open
  5.3 yet.
- **Ayatsuri API**: Cubism Editor's External Application Integration API on a
  port you choose (22033 by default). Apps must be allowed before they can
  drive anything, and the API window shows connected apps and live traffic.
- **Never loses work**: a crash undoes the edit it interrupted, saves a
  recovery copy and a full log, and the next start offers to recover it.
  Autosave every two minutes.

### The app

<p align="center">
  <img src="docs/media/dashboard.png" alt="The Ayatsuri2D home screen with recent models" width="900">
</p>
<p align="center">Model art (left) by <a href="https://www.pixiv.net/users/99163365"><b>マパ*MAPA</b></a> (pixiv ID 99163365), used with permission. Thank you!</p>

- Dockable, rearrangeable panels and saved workspaces.
- Light and dark themes, your own primary colour, interface scale, density and
  rounded corners, reduce motion.
- 30 rebindable shortcuts with Cubism's defaults.
- English and Japanese installer, optional Discord Rich Presence.

## Status

**Beta.** Ayatsuri2D is usable for real rigging today, and it is moving fast.
Save often, keep backups of models you care about, and please
[report what breaks](https://github.com/Yoru-KenomoVT/ayatsuri_2d/issues).

Not built yet:

- model templates and blend shapes (blend shapes are next, in 0.6.0);
- language bindings over the C ABI (the header is there; Unity, Godot and
  Python wrappers are not yet).

The cmo3 export is written from project files alone, without Cubism Editor
to check against: a project it writes reads back here exactly, and the
Editor is the judge of the rest. If it refuses one, please report it.

See the [changelog](CHANGELOG.md) for what each release added.

### Languages

The interface is in English, Afrikaans, Dutch, Japanese, Korean and
Simplified Chinese (Settings > General > Language; it follows your system by default). The
translations were made with tools (AI and machine translation such as Google
Translate), so expect some odd wording. Corrections are very welcome: each
language is one JSON file in [`assets/i18n`](assets/i18n), and a file with the
same name in your config folder's `i18n` directory overrides it without a
rebuild. Model data (parameter, part and mesh names) is never translated.

## Why

Live2D Cubism is effectively the only tool for this work, and:

- its editor runs on **Windows and macOS only**;
- its licence restricts using **your own exported files** with competing
  software;
- its runtime reads offsets out of model files and follows them **without
  bounds checking**:
  [CVE-2023-27566](https://undeleted.ronsor.com/live2d-a-security-trainwreck/)
  makes a `.moc3` an arbitrary memory write, and VTuber models are routinely
  downloaded from strangers.

## Two goals

**Never crashes.** Every model file is treated as hostile: every offset and
index is bounds-checked once, at the import boundary, before anything uses it.
The known-malicious CVE model is rejected in the test suite, and non-test code
denies `unwrap`, `expect` and unchecked indexing by lint, not by discipline.
Fuzzing in CI is planned.

**Never lags.** Not just fast, *predictable*: a steady 45 fps beats a 60 fps
that hitches every few seconds. Allocation happens at load and the frame loop
allocates nothing. A real 356-mesh model evaluates in about half a
millisecond.

More in [`docs/goals.md`](docs/goals.md).

## The runtime and the C ABI

The long-term product is the **runtime**, the part that makes a puppet move,
embeddable anywhere that can call C: Unity, Unreal, Godot, Python,
JavaScript, or an engine of your own. A frame is three calls, set, update,
draw:

```c
#include <ayatsuri.h>

ay2d_model *m = ay2d_load(bytes, len);            /* .aom or .moc3; NULL on bad input */
uint32_t ax = (uint32_t)ay2d_parameter_index(m, "ParamAngleX");
ay2d_set_parameters(m, &ax, &value, 1);           /* arrays in, from any thread      */
ay2d_update(m, dt);                               /* motions, expressions, physics   */
uint32_t n = ay2d_drawables(m, NULL, 0);          /* back to front, views, no copies */
ay2d_drawables(m, out, n);
```

It plays the model's motions and expressions, runs its physics, loads a
Cubism bundle file by file, tells a GPU host which meshes moved, and lets a
tracking thread, a frame thread and a render thread share one model.
`cargo xtask capi` builds the shared and static libraries with the header
generated from the Rust source into `target/capi/`, then links and runs the
C host in `crates/capi/examples/host.c` against them. The developer
reference is [`crates/capi/docs/reference.md`](crates/capi/docs/reference.md).
The crate is Apache-2.0 so a closed game can embed it; the editor stays GPL.

## Build from source

You need a recent stable [Rust](https://rustup.rs) toolchain.

```sh
git clone https://github.com/Yoru-KenomoVT/ayatsuri_2d.git
cd ayatsuri_2d
cargo run --release -p ayatsuri-launcher   # the launcher, which starts the editor
cargo run --release -p ayatsuri            # or the editor on its own
cargo xtask ci                             # everything CI checks
```

Packaging scripts for every platform are in [`packaging/`](packaging).

<details>
<summary><b>Project layout</b></summary>

```
crates/format       ayatsuri-format   the model types and the .aom format
crates/core         ayatsuri-core     deformation, physics, masking, animation,
                                      draw order, editing; no I/O, no renderer
crates/import       ayatsuri-import   moc3, model3/physics3/motion3/exp3 JSON,
                                      PSD, Krita, Clip Studio and cmo3 in;
                                      moc3 and cmo3 out
crates/openmoc3     openmoc3          standalone safe reader for .moc3 files
crates/render       ayatsuri-render   the GPU renderer
crates/editor       ayatsuri-editor   the editor: panels, canvas, windows, API
crates/capi         ayatsuri-capi     the C ABI: ayatsuri.h, shared and static libs
apps/ayatsuri                         the editor app
apps/ayatsuri-launcher                the launcher and splash
packaging/                            Windows, macOS and Linux packages
xtask                                 cargo xtask ci | layers | credits | capi
```

`ayatsuri-core` never depends on `openmoc3`: the moc3 parser is the
hostile-input surface and stays isolated, and `openmoc3` is useful on its own.

</details>

## Documentation

The user guide and tutorials are on the website:
[**ayatsuri-2d.org/docs**](https://ayatsuri-2d.org/docs/) and
[**ayatsuri-2d.org/tutorials**](https://ayatsuri-2d.org/tutorials/).
The technical documents live in this repo:

| | |
|---|---|
| [`docs/goals.md`](docs/goals.md) | what the project guarantees, and how it is tested |
| [`docs/architecture.md`](docs/architecture.md) | the crates and how they depend on each other |
| [`docs/format.md`](docs/format.md) | the model format, and moc3 compatibility |
| [`docs/aom-format.md`](docs/aom-format.md) | the `.aom` file format, normative |
| [`docs/physics.md`](docs/physics.md) | how hair, cloth and accessories move |
| [`docs/integration.md`](docs/integration.md) | embedding the runtime, and the plugin protocol |
| [`docs/provenance.md`](docs/provenance.md) | where the format knowledge came from |

## Contributing

Bug reports, ideas, test results and translations are as welcome as code.
See [CONTRIBUTING.md](CONTRIBUTING.md) for how, and the
[code of conduct](CODE_OF_CONDUCT.md) for how we treat each other.

For ideas, questions and showing off what you rig, come talk on
[Discord](https://discord.gg/WyzDsXzdnh). Bugs still go to
[Issues](https://github.com/Yoru-KenomoVT/ayatsuri_2d/issues), with your log attached.

## Support the project

Ayatsuri2D is free and always will be. If it saves you time or money, you can
help keep it going:

<p>
  <a href="https://buymeacoffee.com/ChaceBerry"><img alt="Buy Me a Coffee" src="https://img.shields.io/badge/Buy%20Me%20a%20Coffee-support-FFDD00?style=for-the-badge&logo=buymeacoffee&logoColor=black"></a>
</p>

Reporting bugs, sharing what you rig with it, and starring the repo help too.

## Licensing

Deliberately not uniform: the project is **GPL-3.0**, `openmoc3` is **MIT** so
the format work flows back to the community it came from, and the C ABI is
**Apache-2.0**. The runtime's licence is still an open question, because a
copyleft runtime cannot be embedded in a proprietary game. See
[`LICENSING.md`](LICENSING.md).

## Artwork

The model in the top, Mesh Edit, rigging and home screen screenshots is drawn by
**マパ\*MAPA** ([pixiv profile](https://www.pixiv.net/users/99163365), ID
99163365), who kindly let us use their PSD to show the editor. Thank you!
The artwork remains theirs: it is not covered by this repository's licences,
so please don't reuse it without their permission.

## Provenance

No Live2D software has been installed, no Live2D binary disassembled, and no
contributor has accepted a Live2D licence agreement. Format knowledge comes
from Live2D's own published specifications, permissively licensed community
work, and independent analysis of model files obtained under their creators'
terms. See [`docs/provenance.md`](docs/provenance.md).

Standing on the shoulders of
[Sakura Motion / PurismCore](https://github.com/SakuraMotion/PurismCore) and
[OpenL2D](https://github.com/OpenL2D/moc3ingbird), both of which corrected our
own findings more than once.

---

<sub>Made by **Yoru_KenomoVT**. Live2D and Cubism are trademarks of Live2D Inc.
This project is not affiliated with, endorsed by, or supported by them. Please
do not contact Live2D about it.</sub>
