# Licensing

This repository is not uniformly licensed, deliberately.

    LICENSE                     GPL-3.0       applies to the project generally
    crates/openmoc3/LICENSE     MIT           see crates/openmoc3/NOTICE
    crates/core/LICENSE         MIT           the runtime
    crates/format/LICENSE       MIT           the model types it runs
    crates/import/LICENSE       MIT           the loaders the C ABI uses
    crates/capi/LICENSE         Apache-2.0    the C ABI

Screenshots in `docs/media` that show third-party artwork are not covered by
any of these: the art belongs to its artist and is used with permission
(see Artwork in README.md for who drew what).

`openmoc3` is a standalone reader for Live2D `.moc3` files. It is MIT because
it derives in part from [PurismCore](https://github.com/SakuraMotion/PurismCore),
which is MIT, and because it is useful to people who will never adopt the rest
of this project. Keeping it permissive means the work flows back to the
community it came from.

## ffmpeg is not part of Ayatsuri2D

No ffmpeg ships in any download. When video export needs one, the editor
or an installer fetches a build straight from the people who make it
([BtbN/FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds) on Windows,
[martin-riedl.de](https://ffmpeg.martin-riedl.de) on Linux and macOS) at the
user's request. It runs as a separate program under its own licence (GPL for
these builds), and the codecs it carries are between the user and ffmpeg.

## The C ABI is Apache-2.0

Settled. `ayatsuri-capi` is Apache-2.0, chosen over MIT for the explicit patent
grant: a boundary that other people's products link against is exactly where an
unstated patent position is worth removing.

## The runtime is MIT

Settled in 0.6.0. `ayatsuri-core` (the runtime), `ayatsuri-format` (the model
it runs) and `ayatsuri-import` (the `.aom` and `.moc3` loaders) are MIT. These
are everything the C ABI links, so a closed game or app can embed the whole
runtime, not only its header.

The reason is linking. Anything linking the ABI links the runtime
transitively, and a GPL-3.0 runtime would make the combined work GPL-3.0: a
permissive wrapper around a copyleft core is permissive in name only. A puppet
runtime nothing can embed is a runtime nobody uses, and VTube Studio and
proprietary games are the embedders that make it worth building.

Parts of the runtime are ported from PurismCore (MIT), whose copyright notice
each of these crates' `LICENSE` keeps (see `docs/provenance.md`).

The editor is a separate and easier case. Nothing links an application, so
copyleft there costs nothing and does protect against a proprietary fork.

See `docs/04-crate-layout.md` and `docs/06-language-split.md` for why the C ABI
is treated as the load-bearing piece.
