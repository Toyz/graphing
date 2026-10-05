# graphing plan

A visual graph and diagram builder. Mermaid's text-first convenience, Visio's
freeform canvas, none of the upsell. Native, fast, extendable.

## Goals

- Clean visual editor that is pleasant for daily work diagrams: flow charts,
  architecture, sequence-ish, org/tree, state machines, ER-ish.
- One plain-text source of truth (`.gph`) that edits two ways: the canvas
  rewrites the text, the text updates the canvas. Diffs well in git.
- Freeform first. You place things; snapping, guides and one-shot auto-layout
  help. Positions are stored in the file.
- Extendable everywhere it gets annoying: stencils, layouts, importers,
  exporters, commands and themes come from packs and Rune plugins.
- Core crates have no UI dependency so notesy (egui) can embed rendering later.

## Non-goals (for now)

- Markdown embedding in other apps. Planned (see Interop) but not built until
  the core crates are stable enough for notesy to depend on.
- Live collaboration. The model is shaped for it (ops, stable ids); the
  transport is a stub.
- Pixel-perfect Visio fidelity on import. Best effort, shapes map to nearest
  stencil.

## Stack

| Layer | Choice |
| --- | --- |
| UI | `gpui-kit = 0.7` (gpui-pre 0.3.7 snapshot + gpui-base + gpui-component) |
| Scripting | `rune = "=0.14.2"`, same sandbox model as notesy |
| Export | `resvg`/`tiny-skia` for PNG; SVG written directly |
| Edition | Rust 2024, workspace `crates/*` like notesy |

Canvas painting is custom (gpui `canvas` + paths). Shell chrome (dock, palette,
inspector, inputs, color picker, settings) uses gpui-component.

## Crates

| Crate | Purpose | Status |
| --- | --- | --- |
| graphing-model | Diagram model: nodes, edges, groups, styles, layout, ids, `Op` edits | phase 0 |
| graphing-dsl | Lossless `.gph` lexer/parser, lowering to model, span patching | phase 0 |
| graphing-stencil | Stencil defs and packs (shape paths, ports, default style, text box) | phase 1 |
| graphing-scene | Model + stencils -> resolved geometry (shapes, paths, text runs). Renderer agnostic | phase 1 |
| graphing-route | Edge routing: straight, orthogonal, curved; obstacle aware | phase 1 |
| graphing-layout | One-shot auto layouts: layered, tree, grid, force | phase 2 |
| graphing-export | SVG, PNG | phase 1 |
| graphing-import | mermaid, draw.io, vsdx | phase 2-3 |
| graphing-plugin | Manifest, permissions, grants, net allowlist (port of notesy-plugin) | phase 2 |
| graphing-script | Rune runtime, one sandboxed VM thread per plugin (port of notesy-script) | phase 2 |
| graphing-sync | Op log + transport trait; stub only | phase 3 |
| graphing-app | gpui shell: windows, canvas, panels, commands, themes | phase 0 |
| graphing-bin | `graphing` executable and CLI (`open`, `render`, `import`, `plugin`) | phase 0 |

Dependency rule: nothing below `graphing-app` depends on gpui. `graphing-scene` +
`graphing-export` are what notesy would pull in for embedded rendering.

## The .gph format (v0)

```graphing
# comments survive every edit
diagram "Auth flow"
use core, flow

style service { fill: "#e8eefc", stroke: "#3b5bdb" }

user: actor "User"
api:  flow.process "API Gateway" .service
db:   db "Postgres" { fill: "#fff4e6" }
cache: "Redis"                      # default stencil (rect)

group backend "Backend" { api db cache }

user -> api "login"
api -> db "lookup" { line: dashed }
api <-> cache
login2: user -> api "refresh"       # named edge

layout {
  user    40 180
  api     320 160 180x64
  db      620 120
  cache   620 260
  backend 280 80 520x280
  user -> api via 180 120, 260 120
}
```

Rules:

- Statements are line oriented; `{ }` blocks may span lines.
- Node: `id: [stencil] ["label"] [.class ...] [{ props }]`.
- Edge: `[id:] a ARROW b [ARROW c ...] ["label"] [.class ...] [{ props }]`.
  Arrows: `->`, `<-`, `<->`, `--`. Line style is a prop, not arrow syntax.
- Props: `key: value` separated by `,` or newline. Values: string, number,
  `#hex` color, ident.
- `layout { }` holds every geometric fact: position, size, edge waypoints.
  Moving things on the canvas only touches this block, so semantic lines stay
  stable in diffs. A node missing from `layout` is placed automatically on
  load and written on first save.
- Unknown statements and props are kept verbatim and reported as warnings,
  so newer files open in older builds.

Lossless editing: the parser keeps byte spans for every statement and value.
Canvas edits become `Op`s, each op becomes minimal text splices (replace a
span, insert a line, remove a line), then the text is reparsed. Comments,
ordering and formatting the user wrote are never rewritten. Same idea as
`toml_edit`.

## Model and edits

- Stable string ids (from the file) for nodes, edges, groups.
- All mutation goes through `Op` (`AddNode`, `RemoveNode`, `SetLabel`,
  `SetProp`, `Move`, `Resize`, `AddEdge`, `SetWaypoints`, ...). Undo/redo is
  an op log with inverses.
- The same ops are what `graphing-sync` would broadcast later. Ops address
  things by id, never by index, so they commute well enough for a CRDT layer
  to wrap them.

## Stencils (the annoying part, made extendable)

A stencil pack is a folder:

```
packs/aws/
  pack.toml          # id, name, version, stencils = [...]
  stencils/lambda.toml
  icons/lambda.svg
```

A stencil declares: outline (built-in shape name or SVG path in a unit box),
optional icon, ports (named anchor points), text box inset, default size and
default style, and resize behaviour (free, keep aspect, fixed). Built-in
`core` pack covers rect, rounded, ellipse, diamond, cylinder, parallelogram,
hexagon, document, actor, note, cloud, container. Packs are pure data; a Rune
plugin can also register stencils whose outline is computed (e.g. a table
shape that grows with rows).

## Plugins (Rune)

Port notesy's model with graphing-specific modules:

- `plugins/<id>/plugin.toml` + `main.rn`, grants in `plugins/grants.toml`.
- Context built with no default Rune modules; only granted host modules are
  installed, so unused permissions fail at compile time.
- One VM thread per plugin, channel messages to the app, instruction and
  memory budgets, auto-stop on repeated faults, hot reload of linked plugins.
- Host modules: `graphing::{log, events, store, settings, json, toml}` always;
  gated: `doc` (read/write model via ops), `commands`, `stencils`, `layout`
  (register a layout algorithm), `import`/`export` (register formats),
  `ui` (panels, inspector sections, status items), `net` (allowlisted hosts),
  `clipboard`, `notify`.
- Example plugins to ship: generate a diagram from a `docker-compose.yml`,
  a Postgres schema ER importer (net), a custom layout.

## Interop

| Feature | Phase |
| --- | --- |
| SVG export | 1 |
| PNG export | 1 |
| Mermaid import (flowchart, then state, class, ER) | 2 |
| draw.io import (`.drawio`, uncompressed + deflate) | 2 |
| Visio import (`.vsdx`, zip + XML, masters -> stencils) | 3 |
| CLI `graphing render in.gph -o out.svg` | 1 |
| Markdown embed (fenced `graphing` blocks, crate notesy can use) | later, planned |
| Share/publish (self-contained HTML, push to configured endpoint) | 3 |

## Power features (backlog)

Command palette (all actions, plugin commands), rebindable keys, themes
(light/dark + plugin themes), multi-select, marquee, align/distribute,
snap to grid and to other nodes, smart guides, copy/paste as `.gph` text,
duplicate, z-order, groups/containers with auto-fit, style presets, format
painter, minimap, search/jump to node, text side panel (split view of the
source), outline tree, templates, autosave and file watching, recent files.

## Phases

0. Skeleton (done 2026-10-04). Workspace, plan, model, DSL parse/lower/patch
   with tests, gpui window that opens a file, draws built-in shapes and
   straight edges, pan, zoom, drag nodes/groups and write positions back to
   text, undo/redo, delete, save, live reload when the file changes on disk,
   `graphing check FILE`. Scene code lives in `graphing-app/src/scene.rs` until
   it moves to `graphing-scene` in phase 1.
1. Real editor (mostly done 2026-10-04: graphing-ui design system, custom
   titlebar with tabs and menus, library/outline panel, inspector, source
   split, floating toolbar, command palette, keymap + settings.json,
   multi-select, marquee, connect handles, resize, inline rename, clipboard,
   align/distribute, SVG/PNG export, mermaid + draw.io import, CLI
   render/import. Orthogonal routing (`route.rs`), notation packs and the
   minimap are in too).
   Original scope: Stencils + core pack, scene crate, routing (straight, ortho),
   selection/multi-select, inspector, create/delete/connect, undo, palette,
   source split view, SVG/PNG export, CLI render.
2. Extensibility. Plugin + script crates, stencil packs on disk, layouts,
   mermaid and draw.io import, theme system. Done 2026-10-04: shape registry,
   JSON packs, Rune plugins, source editor (token roles, completion context
   and folds in `graphing_dsl::lang`; highlighter, live diagnostics and
   completions in `graphing-app/src/source.rs`), groups as edge endpoints
   with handles, resize, member picker and full style. Undo history holds op
   inverses and source-panel text bursts in one stack. Dock (gpui-component's
   engine, own skin in `graphing-app/src/dock.rs` + `graphing_ui::dock`):
   diagrams are center tabs that split, tools are panes in left/right/bottom
   docks, layout persists in state.json. Group designs (`look:`) and pack
   group kinds (containers, SysML, compute, networking); status bar replaced
   by titlebar breadcrumb, Problems badge, canvas HUD and toasts. Settings tab (appearance,
   canvas grid/snap, startup panels, shortcut recorder with conflict hints,
   extensions) and SysML v2 text export/import are in. Pictures (image
   nodes, `.gphz` packages, see `format.md`), confirmation dialogs, color
   picker. Then: notation packs (`notations.md`), templates, guided
   animations with GIF/WebM/APNG/SVG export (`animation.md`), timing
   diagrams, radial mind maps, orthogonal routing, minimap, Visio `.vsdx`
   import (first page: shapes, groups, connectors).
3. Reach. Share/publish, sync stub fleshed out, markdown embed
   crate for notesy.
