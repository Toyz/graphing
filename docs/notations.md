# Notations

Status: all packs below ship, with an example each in `examples/notations/`
(tested to parse clean, resolve every shape and kind, and export). The
Shapes pane lists the notations a diagram uses first, with a picker for the
rest. File > New from Template starts from any of them. Mind maps lay out
around their centre; timing diagrams draw WaveDrom-style waves.

graphing should cover the diagrams people actually draw, each one as
first-party as SysML: real shapes, proper line ends, containers, sensible
defaults, an example to start from. Every notation is a pack
(`crates/graphing-scene/packs/<id>.json`), so users extend them the same way.

## Engine work the notations need

Packs can already draw any SVG path outline, compartments, fields, ports,
stereotypes and group kinds. Missing pieces, added once and used everywhere:

| Feature | Stencil field | Needed by |
| --- | --- | --- |
| Inner strokes drawn over the outline | `detail` (path, same box as the outline) | BPMN gateways and events, fault tree gates, DFD store, ER weak entity |
| Filled marks inside the outline | `mark` (path) | BPMN end/terminate, UML junction, fault tree transfer |
| An icon inside the shape | `glyph: { icon, at, size }` (`center`, `top-left`, `top-right`) | Network devices, ArchiMate layers, BPMN task types, UML component |
| Label under the shape | `label: "below"` | BPMN events and gateways, network devices, FTA gates |
| Heavier outline | `weight` (stroke width) | BPMN end event, call activity |
| Crow's foot ends | `one`, `many`, `one-only`, `zero-one`, `one-many`, `zero-many` | ER |
| Label area, notes | `label_area: [x, y, w, h]`, `notes: true` | C4 person, cards, org chart |
| Top-down and tree layout | `flow: down` (diagram or diagram kind) | Fault trees, org charts |

Canvas, SVG/PNG export and the library tiles draw all of them.

## Packs

| Pack | Shapes | Containers | Line kinds | Diagram kinds |
| --- | --- | --- | --- | --- |
| `c4` | Person, Software system, Container, Database, Queue, Component, External system/person | Enterprise, System, Container boundary | uses, async | Context, Container, Component, Deployment |
| `uml` | Class, Abstract class, Interface, Enumeration, Object, Component, Node, Artifact, State, Choice, Junction, History, Entry/Exit point, Signal send/receive | Package, Subsystem | inheritance, implements, association (directed), dependency, usage, message | Class, Object, Component, Deployment |
| `er` | Entity (columns), Weak entity, View, Enum; Chen: Relationship, Attribute, Key attribute, Multivalued | Schema | one-to-one, one-to-many, many-to-many, optional variants | ER |
| `dfd` | Process, External entity, Data store, Multi-process | Trust boundary, Machine, Network zone | data flow | Data flow, Threat model |
| `control` | Gain, Sum, Integrator, Derivative, Transfer function, Delay, Saturation, PID, Plant, Sensor, Step/Sine source, Scope, Mux, Demux | Subsystem | signal, feedback | Block diagram |
| `fta` | Top/intermediate event, Basic event, Undeveloped, House, Conditioning; AND, OR, XOR, Voting, Priority AND, Inhibit gates; Transfer in/out | none | fault link | Fault tree |
| `bpmn` | Start/intermediate/end events (plain, message, timer, error, signal), Task (user, service, script, send, receive, manual), Subprocess, Call activity, Exclusive/Parallel/Inclusive/Event gateways, Data object, Data store | Pool, Lane, Group | sequence, default, message, data association | Process, Collaboration |
| `es` (event storming) | Domain event, Command, Aggregate, Policy, Read model, External system, Actor, Hotspot, UI | Bounded context | triggers | Event storming |
| `net` | Router, Switch, Firewall, Load balancer, Server, Desktop, Laptop, Phone, Access point, Cloud, Internet, Database, Storage, DNS, CDN, VPN, Queue, IoT | (infra pack) | link, wireless | Network |
| `archimate` | Business actor/role/process/function/event/service/object, Application component/service/interface/function/data, Node, Device, System software, Artifact, Network, Stakeholder, Driver, Goal, Requirement, Principle | Grouping, Location | serving, realization, assignment, access, flow, triggering, influence | Business, Application, Technology, Motivation |
| `org` | Person card, Team, Role; Mind map central topic, Topic, Subtopic | Department | reports-to, branch | Org chart, Mind map |
| `timing` | Signal, Clock, Bus, drawn from `wave: "0.1..p.x=.z"` and `data: [..]` | Signal group | causes | Timing |

## Around the packs

1. One example per notation in `examples/notations/`, also offered as
   templates (File > New from Template), so nobody starts from a blank page.
2. The Shapes pane lists the packs a diagram `use`s first.
3. Tests: every built-in stencil builds a scene and exports to SVG; every
   example parses with no diagnostics and round-trips.

## Later

- Vendor cloud icon sets (AWS, Azure, GCP) carry their own terms; they belong
  in user packs, not built in.
