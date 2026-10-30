# Contributing to Ayatsuri2D

Thank you for wanting to help. You do not need to write Rust to make a
difference: bug reports, feature ideas, testing builds and translations
all move the project forward. Everyone who takes part is asked to follow
the [code of conduct](CODE_OF_CONDUCT.md).

## Reporting a bug

[Open an issue](https://github.com/Yoru-KenomoVT/ayatsuri_2d/issues), or
email [support@ayatsuri-2d.org](mailto:support@ayatsuri-2d.org), with:

- what you did, what you expected, and what happened instead;
- your OS and the version in Help > About;
- the log file if anything went wrong (Settings shows where it is, with a
  Copy path button; set the log level to Debug and repeat the steps for
  the most detail);
- the model, only if it is yours to share. A cut-down model that shows the
  problem is better than a whole one.

Crashes and models that will not open are the most important reports we
get. Please do not hold back because you are not sure it is a bug.

**Security problems** (a file that crashes the app or makes it misbehave on
purpose) should not go in a public issue. Email
[support@ayatsuri-2d.org](mailto:support@ayatsuri-2d.org), and we will fix
it before it is talked about.

If you are expecting a reply to an email, check your spam folder.

## Asking for a feature

Open an issue with one feature per issue. Say what you are trying to do,
not only what you want added: "the art shrinks when the hair swings" is
more useful than "add extended interpolation", because there may be a
better answer. If it is something Cubism Editor has, say how it behaves
there.

## Testing builds

Before a release we sometimes share test builds on the issue they fix.
Trying one and saying whether it does what you asked for is one of the
most useful things you can do.

## Translations

The interface is translated into Afrikaans, Dutch, Japanese, Korean and
Simplified Chinese, mostly with tools, so corrections are very welcome.
Each language is one JSON file in [`assets/i18n`](assets/i18n): the key is
the English text, the value the translation. Fix a line and open a pull
request. To try a change without building, put a file with the same name
in your config folder's `i18n` directory. A new language is a new file.

## Code

### Before you start

Comment on the issue you want to work on (or open one), so nobody does the
same work twice and we can agree on the approach first. For anything big,
talk it through before writing it.

Work happens on the `dev` branch; `main` only moves at a release. Branch
from `dev` and open your pull request against `dev`.

### Building

See [Build from source](README.md#build-from-source) in the README. Then:

```sh
cargo run -p ayatsuri            # the editor
cargo xtask ci                   # everything CI checks, before you push
```

`cargo xtask ci` runs `cargo fmt --check`, clippy on the whole workspace
with warnings as errors, every test, and the C ABI header check.

### The rules the code keeps

- **Never crash.** `unwrap`, `expect`, `panic!` and unchecked indexing are
  denied by lint outside tests. Use `get`, `?` and early returns. Treat
  every file as hostile: check sizes and offsets once, at the import
  boundary.
- **Never lag.** The frame loop allocates nothing; allocate at load.
- **Every word on screen is translatable.** Wrap UI text in `tr(...)` and
  add the string to all five files in `assets/i18n` in the same pull
  request.
- **Match what is around you.** Follow the surrounding code's naming,
  comments and structure; `rustfmt` settles the formatting.
- **Tests for behaviour.** A fix comes with a test that fails without it
  where that is practical; anything that changes how a model evaluates
  needs one.

### Commits and pull requests

Commit messages say what changed in plain words, with the area in front:

```
fix(view): a middle-button drag only pans the view
feat(parameters): extended interpolation, as in Cubism

Fixes #21
```

Use `Fixes #N` when the commit closes an issue and `Refs #N` when it is
part of one. Add a line to [`CHANGELOG.md`](CHANGELOG.md) under the
coming release for anything a user would notice.

### No AI-generated contributions

Pull requests, issues and comments must be your own work. Code written by
AI tools (ChatGPT, Claude, Copilot and the like) is not accepted: if you
could not have made the fix without one, please do not submit it, and open
an issue describing the problem instead. Bug reports and feature requests
go in your own words too, about something you saw yourself; reports that
read like generated text are closed without a review.

Translation tools are fine for writing in English when it is not your
first language. Short and real beats long and polished.

### Clean room

Ayatsuri2D is written from public documentation and from files people own.
Please do not contribute:

- code, headers or binaries from the Live2D Cubism SDK or Cubism Core, or
  anything decompiled from Live2D software;
- Live2D's sample models, or any model or art you do not own or have
  permission to use, in tests, screenshots or anywhere else.

How a Cubism feature behaves may be taken from Live2D's public manual.

## Licensing

The project is GPL-3.0, the C ABI (`crates/capi`) is Apache-2.0, and
`crates/openmoc3` is MIT; see [LICENSING.md](LICENSING.md). By opening a
pull request you agree that your contribution is released under the
licence of the files it changes. There is no CLA.

## Credit

Everyone who helps is credited: code through the commit history, and bug
reports, ideas, testing and translations in the release notes of the
release they landed in.
