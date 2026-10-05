# Animation: guided explainers

A diagram can carry a sequence of steps that walk through it: things appear
in order, dots run along the lines that matter, the camera moves in, and a
caption names each step. Play it on the canvas, or export it as a GIF, an
animated PNG or an animated SVG.

## In the file

Steps live in an `animate { }` block, so they diff and merge like the rest of
the diagram:

```graphing
animate {
  step "Client" 2s {
    show client
    focus client
    highlight client
  }
  step "API server" 2.5s {
    show api
    flow client -> api
    highlight api
  }
  step "The whole picture" {
    focus all
  }
}
```

| Line | Does |
| --- | --- |
| `show a, b` | Fades them in. Anything a `show` names is hidden before its step. A group brings its members. |
| `hide a` | Fades it out. |
| `flow a -> b, e1` | Runs dots along those edges for the step. |
| `highlight a` | Makes it glow for the step. |
| `focus a, b` / `focus all` | Moves the camera onto them, or back out to everything. |
| `move a 300 120` | Slides a shape (a group: everything in it) to a new place; lines re-route as it goes. |
| `ease bounce` | How the step's fades, camera and moves run: `smooth` (default), `linear`, `snappy`, `bounce`. |

A step's length is optional (`2s`, `500ms`; 2 seconds by default). Edges fade
with their ends, so none ever dangles. Targets are node, group or edge ids,
or `a -> b` for an unnamed edge.

## In the app

- **Sequence strip** (under the canvas; the clapper button or View >
  Sequence): one numbered chip per step.
  - Click a chip to preview that step, or double-click it to rename.
  - Right-click a chip to add the selection to it (show, flow, highlight,
    focus), set its length, move it or delete it.
- **+** in the strip, or Ctrl+Shift+N, makes a step from the selection. It
  shows and highlights the selected shapes, and runs dots along the lines
  that reach them.
- **Playback:** Space or F5 plays and pauses. Left and right walk the steps
  while previewing. Escape, or a click on the canvas, goes back to editing.
- **Moves:** while a step is previewed, drag a shape. The move is recorded
  in that step, and the layout stays as it was. The chip's menu clears a
  step's moves and sets its motion.
- **Export:** File > Export Animation. Pick `.gif`, `.webm` (video), `.png`
  (animated) or `.svg` (animated) by the name you save as.

## CLI

```
graphing render explainer.gph -o explainer.gif              # GIF
graphing render explainer.gph -o explainer.webm             # AV1 video
graphing render explainer.gph -o explainer.png --animate    # animated PNG
graphing render explainer.gph -o explainer.svg --animate    # animated SVG (plays in browsers)
```

## How it works

- `graphing_scene::anim::Timeline` turns the steps into a pure function of
  time: per-element opacity, flow offsets, glow, camera and caption. The
  canvas, the SVG writer and the frame renderer all read the same state, so
  playback and export match.
- GIF and APNG frames render with resvg at 15 frames a second, capped at
  1280 px wide.
  - Each frame stores only the rectangle that changed since the last one.
  - Identical frames merge, and the last frame holds for 1.5 seconds before
    the loop.
- Animated SVG uses SMIL `<animate>` keyframes sampled from the timeline.
  The camera animates the inner `viewBox`, and flow dots are zero-length
  round dashes whose offset animates.
- WebM:
  - AV1 from `rav1e`, at its fastest preset, without its assembly so no
    nasm is needed.
  - BT.709 colour, in our own small Matroska writer.
  - Held frames repeat at almost no cost.
- Frames render on every core. Moving shapes rebuild the scene per frame
  in a fixed viewport that covers every move.
- In animated SVG:
  - Sliding nodes use `animateTransform`.
  - Lines and group frames that change shape are swapped per sample run.
- Encoders: `gif`, `png` (MIT/Apache) and `rav1e` (BSD-2-Clause).

Not yet: MP4 (H.264 carries patent licensing).
