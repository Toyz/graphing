# Contributing to graphing

Thanks for helping. Bug reports, notation requests, shape packs, docs fixes and code are all welcome. Please keep things friendly; the [code of conduct](CODE_OF_CONDUCT.md) is short.

## Reporting a problem

Open an issue; the templates ask for what helps most. In short:

- what you did, what you expected, and what happened;
- the smallest `.gph` file that shows it (the text is the diagram, so paste it);
- your OS and how you run graphing (release build, `cargo run`, the CLI).

For rendering problems, the output of `graphing render file.gph -o out.svg` is often the quickest way to show it.

## Building and checking

You need a recent stable Rust (1.88 or newer). On Linux, gpui also needs the usual X11 or Wayland, xkbcommon and Vulkan development packages.

```sh
cargo run --release                 # the editor
cargo run --release -- file.gph     # open a file
```

Before you open a pull request, both of these must pass:

```sh
cargo clippy --workspace --all-targets -q -- -D warnings
cargo test --workspace -q
```

## How the code is organised

`docs/plan.md` has the full picture. The rules that matter most when you change things:

- **The `.gph` text is the source of truth.**
  - Every edit is a `graphing_model::Op`, applied through `Document::apply`. That splices the text and reparses, so the rest of the file stays as written.
  - Never regenerate a whole file; comments and formatting must survive edits.
- **Every new `Op` or text edit path gets a round-trip test** in `crates/graphing-dsl/src/tests.rs`. The `check` helper applies the op to the text and to the model, compares them, then checks that the inverse restores the original.
- **Geometry lives only in the `layout { }` block** of a file.
- **Nothing below `graphing-app` depends on gpui.** The model, language, scene, exporters and importers stay free of UI code so other programs can embed them.
- **Shapes come from packs, not Rust.**
  - Built-in notations are JSON in `crates/graphing-scene/packs/`.
  - Notation meaning (headers, sections, ports) lives in `crates/graphing-scene/src/notation.rs`.
  - If a notation needs something packs cannot express yet, add it to the pack format: a stencil field, plus drawing it in `paint.rs` and in the SVG exporter. Then document it in `docs/plugins.md`.
- **Gpui imports** come through `gpui_kit::*`; do not add a separate `gpui` dependency.

### The look of the app

- All app chrome is built from `graphing_ui::kit` components and `graphing_ui::tokens`: colors by role through `cx.ui()`, and sizes, radii, gaps and type from the scales.
- Raw `px(..)` and `rgb(..)` values in `graphing-app` fail the `tests/house_style.rs` test. Only `paint.rs`, which draws the canvas in diagram units and user colors, is exempt.
- If a control or look does not exist yet, add a token or kit component in `graphing-ui` first, then use it.

### Testing UI

- Interaction tests use gpui's headless `TestAppContext` (`#[gpui_kit::test]`). See the tests in `crates/graphing-app/src/view/tests.rs` and `dock.rs`.
- For screenshots, debug builds honour `GRAPHING_DEBUG_OPEN`. It can open the palette, a menu or settings, select shapes, preview an animation step, and more; the list is in `docs/debugging.md`.

## Dependencies and licenses

graphing is dual-licensed MIT OR Apache-2.0, and it takes no copyleft code.

- Check a crate's license before adding it. MIT, Apache-2.0, BSD, ISC and Zlib are fine.
- MPL-2.0 is accepted only for crates used unmodified (today, `mp4parse`).
- `crates/graphing-app/build.rs` fails the build on any dependency whose only licenses are copyleft. It warns about licenses it does not recognise, so new ones get a look before they ship.
- The same script lists every crate, with its license text, under Settings > Open Source.

## Pull requests

- Keep each pull request to one change. Large features are easier to land after a short issue or discussion about the approach.
- Write commit messages in the imperative ("Add crow's foot ends"). Say why when it is not obvious.
- Code reads like the code around it: match its naming, comment density and idioms.
- Comments explain why, not what.
- Plain text only in code, comments, docs and commit messages: no emoji.
- Update the docs in `docs/` when you change the format, a pack field, the CLI or user-visible behaviour.
- New notation packs should come with an example in `examples/notations/` and a template entry in `crates/graphing-app/src/templates.rs`. A test parses every example and checks it renders.

## License of contributions

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in graphing by you, as defined in the Apache-2.0 license, shall be dual licensed under the MIT and Apache-2.0 licenses, without any additional terms or conditions.
