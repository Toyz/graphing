# SysML in graphing

Goal: author SysML diagrams as fast as flowcharts, in the same `.gph`
text, with a technical look (mono compartments, thin strokes, frame with
header tab, accent item flows) and SysML v2 textual interop.

Reference look: the HIL bench IBD in the SIE debrief
(`tools/diagrams.py`, `ibd()`): `role : Type` parts with a `parts`
compartment, square proxy ports straddling the border with names outside,
plain connectors, `« flow »` in accent over the item list, and an
`ibd [block] Title [View]` frame tab.

## Status (2026-10-04)

A, B, C and D are in. SysML v2 text: `graphing render x.gph -o x.sysml`
(or File > Export SysML v2) writes it, File > Import and `graphing import`
read it back (`graphing_export::sysml2`, `graphing_import::sysml2`). Ids ride
as short names (`part def <vehicle> Vehicle`), relationships use v2 forms
where they exist (`:>`, `satisfy`, `connect`, `flow`, `transition`,
successions, `message`) and `#kind dependency` otherwise; layout, group
props and other extras ride in `// @` comments, so every example round-trips
(tested). Groups are first-party SysML too: `look: sysml` (block boundary
with `«stereotype»`) and `look: package` (frame tab), and the sysml pack's
group kinds (block, subsystem, system context, package, boundary). Examples: `examples/hil-ibd.gph`
(the debrief IBD, rebuilt) and `examples/sysml/` (bdd, req, act, stm, uc, sd).

## Phases

A. Foundation (generic, any notation benefits)
   - List values: `parts: ["obc : OnboardComputer", "adcs : ADCSProcessor"]`.
   - Ports as edge endpoints: `uut.busPort -- fe.busPort`. Ports render as
     squares on the node border, side chosen toward the other end.
   - Stereotypes: `stereotype: block` renders `«block»` over the name.
   - Diagram frame: `diagram "HIL Test Bench" { kind: ibd, context: block,
     view: Architecture }` renders the frame and header tab.
   - Edge ends: `head` / `tail` = arrow, open, triangle, diamond,
     filled-diamond, circle, none. `kind` shortcuts set them (composition,
     aggregation, generalization, dependency, flow, satisfy, ...).
   - Looks: `look: technical` on the diagram (mono compartments, sharp
     corners, frame). `use sysml` implies it.
B. SysML stencils (`sysml.*`)
   - Structure: block, part, port, interface, constraint, value type,
     package.
   - Requirements: requirement (id, text), satisfy, verify, derive, refine,
     trace, containment.
   - Behavior: action, initial, final, decision, merge, fork, join, object
     node, state (entry/do/exit), transitions `trigger [guard] / effect`.
   - Interaction: actor, use case, system boundary, lifeline, message,
     include / extend.
C. Right sidebar, notesy style
   - Panel registry: one panel at a time, header picker, icon rail, plugins
     add panels.
   - Property inspector: icon + name over an in-place, type-aware value
     editor; per-property menu; one-click adds for known props.
D. SysML v2 text
   - Export: part def / part / port / connect / flow / requirement /
     satisfy / state / action from a diagram.
   - Import: practical subset back to `.gph`.

## Example (target syntax)

```graphing
diagram "HIL Test Bench" { kind: ibd, context: block, view: Architecture }
use sysml

uut: sysml.part "FlightArticle" {
  parts: ["obc : OnboardComputer { flight build, unmodified }", "adcs : ADCSProcessor"]
}
fe: sysml.part "IOFrontEnd" { parts: ["timeMaster : PPSGenerator"] }

uut.busPort -- fe.busPort "SensorFrame, ActuatorCommand" { kind: flow }
```
