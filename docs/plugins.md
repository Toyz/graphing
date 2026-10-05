# Extending graphing: shape packs and Rune plugins

Everything graphing draws comes from packs. The built-in ones
(`crates/graphing-scene/packs/core.json`, `sysml.json`) use the same format
as yours, so anything a built-in shape can do, yours can too.

| | Where | Reload |
| --- | --- | --- |
| Shape packs | `<config>/packs/<id>/pack.json` (or `packs/*.json`) | Reload Shape Packs |
| Plugins | `<config>/plugins/<id>/plugin.json` + script | Reload Plugins |

`<config>` is `~/.config/graphing` on Linux, `~/Library/Application Support/graphing` on macOS and `%APPDATA%\graphing` on Windows.

The CLI loads both folders too, and `--packs DIR` adds more:
`graphing render diagram.gph -o out.svg --packs ./my-packs`.

## Shape packs (JSON, no code)

A pack may carry an `icon` (Lucide name) and a `description`; the Shapes
pane's notation picker shows both. Diagram kinds may set `"flow": "down"` so
new nodes lay out top to bottom (fault trees, org charts), or
`"radial"` around a centre (mind maps); a diagram can also say `flow: down`
or `flow: radial` itself.

```json
{
  "id": "cloud",
  "name": "Cloud",
  "version": "0.1.0",
  "stencils": [
    {
      "name": "function",
      "title": "Function",
      "category": "Cloud",
      "outline": { "path": "M12 0 H88 L100 50 L88 100 H12 L0 50 Z", "view": [0, 0, 100, 100] },
      "size": [150, 60],
      "icon": "Zap",
      "defaults": { "fill": "#fff4e6", "stroke": "#f08c00" },
      "props": [{ "key": "runtime", "label": "Runtime" }],
      "fields": [{ "key": "runtime", "format": "{}" }]
    }
  ],
  "edge_kinds": [
    { "name": "publishes", "group": "Cloud", "description": "Sends an event", "head": "open", "dashed": true, "stereotype": "event" }
  ],
  "diagram_kinds": []
}
```

Files use a stencil as `<pack>.<name>`: `api: cloud.function "upload"`.
A file can give a pack a short name, which helps with long or
look-alike pack ids:

```graphing
use acme.cloud as ac, sysml as s
api: ac.function "upload"
a -> b { kind: s.flow }
```

The diagram keeps the pack's real id (`acme.cloud.function`), so the short
name is the file's own business. Canvas edits write names back in the
file's short form.

Stencil fields (only `name` and `title` are required):

| Field | Meaning |
| --- | --- |
| `category` | Library section; defaults to the pack name |
| `outline` | A built-in outline name (`rect rounded ellipse diamond cylinder parallelogram hexagon note actor block initial final bar package lifeline`) or `{ "path": "<svg path data>", "view": [x, y, w, h] }` |
| `header` | `label` (default), `role` (`id : Label`, SysML part style) or `none` |
| `stereotype` | Shown as `«stereotype»` above the name |
| `compartments` | List props drawn as compartments, in order |
| `fields` | Text lines from props: `{ "key": "rid", "format": "id = \"{}\"", "wrap": 44 }` |
| `props` | What the inspector offers: `{ "key", "label", "kind": "text" \| "longtext" \| "list" }` |
| `size` | Default `[width, height]` |
| `defaults` | Props used when a node sets none (`fill`, `stroke`, `color` ...) |
| `icon` | Lucide icon name for lists and the palette |
| `aliases` | Other names that resolve to this stencil |
| `hidden` | Keep out of the library |
| `detail` | Strokes drawn over the outline, as SVG path data in the outline's box (or `{ "path", "view" }`): gateway crosses, gate curves |
| `mark` | Filled shapes drawn over the outline in the line color: junction dots, terminate discs |
| `glyph` | A Lucide icon inside the shape: `{ "icon": "Router", "at": "center" \| "top-left" \| "top-right" \| "left", "size": 0.5 }` |
| `label` | `inside` (default) or `below`, for small symbols named underneath |
| `label_area` | Where the label sits, as fractions `[x, y, w, h]` of the box |
| `notes` | `true` shows `fields` as smaller centred lines under the label (C4 style) instead of a compartment |
| `weight` | Outline stroke width |
| `render` | `wave`: a timing signal drawn from the node's `wave` and `data` props (WaveDrom letters) instead of an outline |

Edge kinds set `head` / `tail` (`none arrow open triangle diamond
filled-diamond circle`, and the crow's foot ends `one one-only many zero-one
one-many zero-many`), `dashed` and an optional `stereotype`; files use
them as `a -> b { kind: publishes }`. A pack loaded again replaces itself.

Node-graph stencils can list pins, plain (`"name: type"`) or repeated by a
count or over a list prop, and a pack can declare subtypes:

```json
{ "name": "sequence", "title": "Sequence", "outline": "rect",
  "pins": { "in": ["exec"],
            "out": [{ "name": "then {i}", "type": "exec", "count": "outputs", "default": 2 }] } },
{ "name": "switch", "title": "Switch", "outline": "rect",
  "pins": { "in": ["exec", "selection: string"],
            "out": [{ "name": "{item}", "type": "exec", "each": "cases" }, "default: exec"] } }
```

`"types": { "Pawn": "Actor", "Actor": "Object" }` at the top of a pack makes
each type fit where its supertype is taken. See `notations.md` for how
pins, type variables (`T`) and wiring checks behave.

Kind names are unique within a pack, but two packs may share one (C4 and
SysML both have `async`). `kind: c4.async` names one exactly; a bare
`async` means the first pack in the diagram's `use` list that has it, else
the pack loaded last. The inspector writes the qualified form only when a
name is shared.

Group kinds are container presets. They show in the library (one section per
`category`) and in the inspector's Kind picker; files use them as
`group vpc "Prod" { api db } { kind: vpc }`:

```json
"group_kinds": [
  { "name": "vpc", "title": "VPC", "category": "Networking", "look": "zone",
    "icon": "Cloud", "defaults": { "fill": "#4dabf7" }, "description": "Private network" }
]
```

`look` is one of `dashed solid sysml package lane zone card`; `stereotype`
fills the `«»` line of `sysml` groups. `props` are the kind's fields
(`{ "key": "cidr", "label": "CIDR" }`): the inspector offers them and set
values show after the group's name. `defaults` give starting values: style
keys (`fill`, `stroke`, `color`) apply where the group sets none, and field
keys (`"cidr": "10.0.0.0/16"`) are written into a group created from the
library. A group's own `look:` overrides its kind's.
Built-in packs live in `crates/graphing-scene/packs/`: core, sysml, uml,
c4, er, bpmn, dfd, control, fta, archimate, es, org, net and infra (see
`docs/notations.md`, and `examples/notations/` for one diagram each).

See `examples/packs/cloud/` and `examples/cloud.gph`.

## Rune plugins

```
my-plugin/
  plugin.json
  main.rn
  pack.json        # optional static pack
```

```json
{
  "id": "sysml-tools",
  "name": "SysML tools",
  "version": "0.1.0",
  "main": "main.rn",
  "permissions": ["doc.read", "doc.write", "commands", "stencils", "notify"],
  "pack": "pack.json"
}
```

Each plugin runs on its own thread in its own Rune VM, with an instruction
budget per call and a memory limit; it fails to compile if it uses a module
it was not granted. Five failures in a row stop it. Scripts and packs must
stay inside the plugin folder.

`main()` runs once at load. Register commands there; they appear in the
command palette as `<Plugin name>: <title>`, and their edits apply to the
open diagram as one undo step.

| Module | Permission | Functions |
| --- | --- | --- |
| `graphing::log` | always | `info(text)`, `warn(text)`, `error(text)` |
| `graphing` | `notify` | `notify(text)` (status bar) |
| `graphing::stencils` | `stencils` | `register(pack)` (an object shaped like pack.json) |
| `graphing::commands` | `commands` | `register(id, title, fn)` |
| `graphing::doc` | `doc.read` | `title()`, `nodes()`, `edges()`, `groups()`, `selection()` |
| `graphing::doc` | `doc.write` | `add_node(#{ stencil, label, x, y, id, props })` returns the id, `add_edge(#{ from, to, label, kind })`, `set_label(id, text)`, `set_prop(id, key, value)`, `move_to(id, x, y)`, `remove(id)`, `set_selection(ids)` |

Nodes come as objects `#{ id, label, stencil, x, y, props }`, edges as
`#{ id, from, to, label, props }`; missing values are empty text or `0.0`,
never `None`, so comparisons need no unwrapping.

```rune
use graphing::{commands, doc};

pub fn main() {
    commands::register("grid", "Arrange in a grid", grid);
}

pub fn grid() {
    let i = 0;
    for id in doc::selection() {
        doc::move_to(id, ((i % 4) * 260) as f64, ((i / 4) * 160) as f64);
        i += 1;
    }
}
```

See `examples/plugins/sysml-tools/` for a complete plugin (registers a
callout shape, numbers requirements, lays out a grid).
