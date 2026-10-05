# graphing: notes for coding agents

A visual diagram editor on gpui (through `gpui-kit`) with Rune plugins.
Read `docs/plan.md` first for the goals, the crate map, the `.gph` format and
the phases. Human contributors: `CONTRIBUTING.md` says the same things at
more length.

## Checks (both must pass before calling work done)

```
cargo clippy --workspace --all-targets -q -- -D warnings
cargo test --workspace -q
```

## Rules

- Plain text only: no emoji in code, comments, docs or commit messages.
- Nothing below `graphing-app` may depend on gpui. Model, DSL, scene, export
  and import stay UI-free so other programs can embed them.
- The `.gph` text is the source of truth.
  - All edits go through `graphing_model::Op` -> `Document::apply`, which
    splices the text and reparses.
  - Never regenerate a whole file; comments and formatting must survive.
- Every new `Op` or text edit path gets a round-trip case in
  `crates/graphing-dsl/src/tests.rs`. `check` asserts the model equals the
  reparsed text and that the inverse restores it.
- Geometry lives only in the `layout { }` block.
- `gpui-kit = 0.7` pulls `gpui-pre =0.3.7` (not crates.io `gpui 0.2`). Use
  `gpui_kit::*` imports; do not add a separate `gpui` dependency.
- Check licenses before adding crates (MIT, Apache-2.0, BSD, ISC, Zlib are
  fine). No copyleft:
  - `crates/graphing-app/build.rs` fails the build on any dependency whose
    only licenses are copyleft.
  - MPL-2.0 is the one exception (`mp4parse`), allowed only unmodified.
  - The same script lists every crate, with its license text, under Settings >
    Open Source.
- Shapes, edge kinds, group kinds and diagram kinds come from packs.
  - Built-in packs are JSON in `crates/graphing-scene/packs/`, loaded through
    `crates/graphing-scene/src/stencils.rs`.
  - Never hard-code a shape in Rust; add it to a pack. If a pack cannot
    express it yet, add a stencil field, draw it in `paint.rs` and the SVG
    exporter, and document it in `docs/plugins.md`.
  - Notation meaning (headers, sections, ports) lives in `notation.rs`.
- Destructive actions confirm through `confirm.rs` (`Workspace::ask`), never a
  native prompt.

## Design system (`graphing-ui`)

- All chrome is built from `graphing_ui::kit` components and
  `graphing_ui::tokens`: colors by role through `cx.ui()`; sizes, radii, gaps
  and type from the scales.
- Never style with raw `px(..)` or `rgb(..)` in `graphing-app`; the
  `tests/house_style.rs` test fails on them. Only `paint.rs` (canvas, diagram
  units, user colors) is exempt.
- Need a new look or control? Add a token or kit component in `graphing-ui`
  first, then use it.
- Palettes are Midnight and Paper with a violet accent.
  `graphing_ui::install(dark, window, cx)` sets the palette and dresses
  gpui-component (menus, inputs, tooltips) to match.
- Icons: any Lucide icon through `graphing_ui::kit::Lucide`; the app ships
  `AllAssets`.

## Where things live

| What | Where |
| --- | --- |
| Files (`.gph`, `.gphz`) | `docs/format.md`; package format in `crates/graphing-package`; app file IO in `graphing-app/src/files.rs` |
| Animation | `docs/animation.md`; timeline in `graphing-scene/src/anim.rs`; strip and playback in `graphing-app/src/sequence.rs` |
| Routing and layout | `graphing-scene/src/route.rs`, `auto_place` in `graphing-scene/src/lib.rs` |
| Notations | `docs/notations.md`, `docs/sysml.md` |
| Dock | `graphing-app/src/dock.rs` (panes and skin over gpui-component's dock); chrome in `graphing_ui::dock`. The layout persists in `state.json`; tests never restore it. |
| Plugins | `crates/graphing-script` (sandboxed threads, permission-gated host modules); app wiring in `graphing-app/src/plugins.rs` |

## Settings

- The config folder is `settings::config_dir()`: `~/.config/graphing` on
  Linux, `~/Library/Application Support/graphing` on macOS,
  `%APPDATA%\graphing` on Windows. `GRAPHING_CONFIG_DIR` overrides it; tests
  set it so they never touch real settings.
- `settings.json` there holds user settings: theme, panels, canvas,
  keybindings. The app rewrites only keys it owns and keeps the rest.
- `state.json` holds app state (open files, recent files). It is not for
  users.
- Keybindings:
  - Defaults are in `crates/graphing-app/src/keymap.rs`. Use `secondary-`
    (cmd on macOS, ctrl elsewhere) for app shortcuts, never a bare `ctrl-`;
    simulated keystrokes in tests do the same.
  - User entries go in settings `keybindings` as
    `{"keys","action","context","args"}`; `action: null` unbinds.
  - Actions are addressed by gpui name, such as `graphing::AddShape`.

## Testing UI

- Interaction tests use gpui's headless `TestAppContext`
  (`#[gpui_kit::test]`, dev-dependency feature `test-support`). See
  `crates/graphing-app/src/view/tests.rs`, `dock.rs` and `menubar.rs`.
- Startup hooks (`GRAPHING_DEBUG_OPEN`) and how to capture screenshots of only
  graphing's window are in `docs/debugging.md`. Never capture the whole
  screen or "the active window"; that grabs whatever else is open.
