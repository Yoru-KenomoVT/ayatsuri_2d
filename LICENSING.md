# Licensing

This repository is not uniformly licensed, deliberately.

    LICENSE                     GPL-3.0     applies to the project generally
    crates/openmoc3/LICENSE     MIT         see crates/openmoc3/NOTICE

`openmoc3` is a standalone reader for Live2D `.moc3` files. It is MIT because
it derives in part from [PurismCore](https://github.com/SakuraMotion/PurismCore),
which is MIT, and because it is useful to people who will never adopt the rest
of this project. Keeping it permissive means the work flows back to the
community it came from.

## Open question

Whether the **runtime and C ABI** should also be permissive is not yet settled.
The argument for it: a GPL runtime cannot be linked into a proprietary game or
into VTube Studio, which rules out the embedders that make a puppet runtime
worth building. The argument against: copyleft protects the work.

The editor is a different case. Nothing links an application, so copyleft there
costs nothing and does protect against a proprietary fork.

See `docs/04-crate-layout.md` and `docs/06-language-split.md` for why the C ABI
is treated as the load-bearing piece.
