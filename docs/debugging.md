# Debugging and screenshots

## Startup hooks

Debug builds read `GRAPHING_DEBUG_OPEN` at startup and open or select
something, so a screenshot or a manual check starts in the right state. One
hook per run.

| Value | Opens |
| --- | --- |
| `palette` | The command palette |
| `settings` | The Settings tab |
| `settings:<query>` | Settings searched for `query`; `settings:oss=<crate>` also opens that crate in Open Source |
| `templates` | File > New from Template |
| `menu:<n>` | Title bar menu `n` (0 is File); `menu:<n>/<row>` also opens the submenu at that row |
| `select:<id>,<id>` | Selects those shapes |
| `open-select:<picker>` | An inspector picker (`diagram-kind`, `shape`, `edge-kind`, `node-group`, `group-add`, `group-look`, `group-kind`, `library-notation`) |
| `open-color:<key>@<id>` | The color picker for `fill`, `stroke` or `color` of `id` |
| `context:<id>` | The right-click menu on `id` (`context:` alone: the empty canvas) |
| `rename:<id>` | The in-place label editor on `id` |
| `confirm-delete:<id>` | The delete confirmation for `id` |
| `ask-images` | The "add this picture" choice dialog |
| `library:containers` / `library:shapes` | The Shapes pane filtered to one kind |
| `notation:<pack>` / `notation:*` | The Shapes pane showing one notation, or all |
| `step:<n>` | Previews animation step `n` (from 1) |
| `play` | Plays the animation |
| `zoom:<factor>` | Zooms the canvas by `factor` after the first paint (shows the minimap) |
| `drag-bench:<id>` | Drags `id` for 60 frames, prints the time, then quits |

`GRAPHING_TRACE=1` prints how long renders and paints take.

## Screenshots

Capture only graphing's window, never the whole screen or "the active
window": those grab whatever else is open. On Linux:

```sh
env -u WAYLAND_DISPLAY XDG_CONFIG_HOME=$(mktemp -d) \
  GRAPHING_DEBUG_OPEN=step:2 target/debug/graphing examples/animated/request.gph &
id=$(xdotool search --sync --pid $! | tail -1)   # the window, once it exists
sleep 3
import -window "$id" shot.png
```

Running under XWayland (`env -u WAYLAND_DISPLAY`) lets `xdotool` find the
window. A fresh `XDG_CONFIG_HOME` keeps your own settings, recent files and
layout out of the picture. Settings for a shot (hiding a panel, say) go in
`$XDG_CONFIG_HOME/graphing/settings.json`.

## Interaction tests

Synthetic input does not reach real windows on every desktop, so
interactions are tested headless with gpui's `TestAppContext`
(`#[gpui_kit::test]`, dev-dependency feature `test-support`). See the tests
in `crates/graphing-app/src/view/tests.rs`, `dock.rs` and `menubar.rs`.
