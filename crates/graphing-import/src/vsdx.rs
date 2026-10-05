//! Visio (`.vsdx`): a zip of XML. The first page's shapes become nodes
//! (groups become groups), its connectors become edges through the page's
//! `Connect` records, and master names pick graphing shapes.

use std::collections::HashMap;
use std::io::{Cursor, Read as _};

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use crate::{ImportError, OEdge, OGroup, Out, ident};

/// A glued connector end: connector id, `BeginX`/`EndX`, shape id.
type Connect = (String, String, String);

/// Pixels per inch: Visio measures in inches.
const PX: f64 = 96.0;

#[derive(Debug, Default, Clone)]
struct Shape {
    id: String,
    name: String,
    master: Option<String>,
    group: bool,
    /// Pin, size and local pin, in inches.
    pin: (f64, f64),
    size: (f64, f64),
    loc: (f64, f64),
    /// Set on 1-D shapes (connectors).
    begin: Option<(f64, f64)>,
    end: Option<(f64, f64)>,
    text: String,
    children: Vec<Shape>,
}

fn attr(e: &BytesStart, name: &str) -> Option<String> {
    e.attributes().flatten().find(|a| a.key.as_ref() == name).and_then(|a| a.normalized_value(quick_xml::XmlVersion::default()).ok()).map(|v| v.into_owned())
}

fn read(zip: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> Option<String> {
    let mut f = zip.by_name(name).ok()?;
    let mut s = String::new();
    f.read_to_string(&mut s).ok()?;
    Some(s)
}

pub(crate) fn parse(bytes: &[u8]) -> Result<Out, ImportError> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| ImportError::Parse(format!("not a .vsdx: {e}")))?;
    let mut pages: Vec<String> = zip.file_names().filter(|n| n.starts_with("visio/pages/page") && n.ends_with(".xml")).map(String::from).collect();
    pages.sort_by_key(|n| n.trim_start_matches("visio/pages/page").trim_end_matches(".xml").parse::<u32>().unwrap_or(u32::MAX));
    let first = pages.first().ok_or_else(|| ImportError::Parse("the .vsdx has no pages".into()))?.clone();
    let page = read(&mut zip, &first).ok_or_else(|| ImportError::Parse(format!("cannot read {first}")))?;
    let masters = read(&mut zip, "visio/masters/masters.xml").map(|m| masters(&m)).unwrap_or_default();
    let mut out = Out::default();
    if pages.len() > 1 {
        out.warnings.push(format!("{} pages; only the first was imported", pages.len()));
    }
    let (shapes, connects) = page_shapes(&page)?;
    build(&shapes, &connects, &masters, &mut out);
    Ok(out)
}

/// Master id -> its universal name (`Process`, `Decision`...).
fn masters(xml: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut r = Reader::from_str(xml);
    loop {
        match r.read_event() {
            Ok(Event::Start(e) | Event::Empty(e)) if e.name().as_ref() == "Master" => {
                if let Some(id) = attr(&e, "ID") {
                    out.insert(id, attr(&e, "NameU").or_else(|| attr(&e, "Name")).unwrap_or_default());
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

/// The page's top-level shapes (groups holding theirs) and its connects:
/// (connector, end cell `BeginX`/`EndX`, shape).
fn page_shapes(xml: &str) -> Result<(Vec<Shape>, Vec<Connect>), ImportError> {
    let mut r = Reader::from_str(xml);
    let mut stack: Vec<Shape> = Vec::new();
    let mut top: Vec<Shape> = Vec::new();
    let mut connects = Vec::new();
    let mut in_text = 0usize;
    loop {
        let ev = r.read_event().map_err(|e| ImportError::Parse(format!("page XML: {e}")))?;
        match ev {
            Event::Start(e) if e.name().as_ref() == "Shape" => stack.push(shape_of(&e)),
            Event::Empty(e) if e.name().as_ref() == "Shape" => {
                let s = shape_of(&e);
                match stack.last_mut() {
                    Some(p) => p.children.push(s),
                    None => top.push(s),
                }
            }
            Event::End(e) if e.name().as_ref() == "Shape" => {
                let s = stack.pop().expect("open shape");
                match stack.last_mut() {
                    Some(p) => p.children.push(s),
                    None => top.push(s),
                }
            }
            Event::Start(e) | Event::Empty(e) if e.name().as_ref() == "Cell" => {
                // Cells of the shape itself, not of its sections' rows.
                if let (Some(s), Some(n), Some(v)) = (stack.last_mut(), attr(&e, "N"), attr(&e, "V").and_then(|v| v.parse::<f64>().ok())) {
                    match n.as_str() {
                        "PinX" => s.pin.0 = v,
                        "PinY" => s.pin.1 = v,
                        "Width" => s.size.0 = v,
                        "Height" => s.size.1 = v,
                        "LocPinX" => s.loc.0 = v,
                        "LocPinY" => s.loc.1 = v,
                        "BeginX" => s.begin.get_or_insert((0.0, 0.0)).0 = v,
                        "BeginY" => s.begin.get_or_insert((0.0, 0.0)).1 = v,
                        "EndX" => s.end.get_or_insert((0.0, 0.0)).0 = v,
                        "EndY" => s.end.get_or_insert((0.0, 0.0)).1 = v,
                        _ => {}
                    }
                }
            }
            Event::Start(e) if e.name().as_ref() == "Text" => in_text += 1,
            Event::End(e) if e.name().as_ref() == "Text" => in_text = in_text.saturating_sub(1),
            Event::Text(t) if in_text > 0 => {
                if let Some(s) = stack.last_mut() {
                    s.text.push_str(&t.xml_content(quick_xml::XmlVersion::default()));
                }
            }
            // `&amp;` and friends arrive on their own.
            Event::GeneralRef(r) if in_text > 0 => {
                let ch = match r.resolve_char_ref() {
                    Ok(Some(c)) => Some(c),
                    _ => match r.as_ref() {
                        "amp" => Some('&'),
                        "lt" => Some('<'),
                        "gt" => Some('>'),
                        "quot" => Some('"'),
                        "apos" => Some('\''),
                        _ => None,
                    },
                };
                if let (Some(s), Some(c)) = (stack.last_mut(), ch) {
                    s.text.push(c);
                }
            }
            Event::Start(e) | Event::Empty(e) if e.name().as_ref() == "Connect" => {
                if let (Some(from), Some(cell), Some(to)) = (attr(&e, "FromSheet"), attr(&e, "FromCell"), attr(&e, "ToSheet")) {
                    connects.push((from, cell, to));
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok((top, connects))
}

fn shape_of(e: &BytesStart) -> Shape {
    Shape {
        id: attr(e, "ID").unwrap_or_default(),
        name: attr(e, "NameU").or_else(|| attr(e, "Name")).unwrap_or_default(),
        master: attr(e, "Master"),
        group: attr(e, "Type").as_deref() == Some("Group"),
        ..Default::default()
    }
}

/// A graphing shape for a Visio master name.
fn stencil(name: &str) -> Option<&'static str> {
    let n = name.to_lowercase();
    let n = n.split('.').next().unwrap_or(&n).trim();
    Some(match n {
        _ if n.contains("decision") || n.contains("diamond") => "decision",
        _ if n.contains("terminator") || n.contains("start") || n.contains("end") => "terminal",
        _ if n.contains("database") || n.contains("cylinder") || n.contains("stored data") => "db",
        _ if n.contains("data") || n.contains("parallelogram") || n.contains("input") => "io",
        _ if n.contains("preparation") || n.contains("hexagon") => "prep",
        _ if n.contains("document") || n.contains("note") || n.contains("annotation") => "note",
        _ if n.contains("ellipse") || n.contains("circle") || n.contains("oval") => "ellipse",
        _ if n.contains("actor") || n.contains("person") || n.contains("user") => "actor",
        _ if n.contains("process") || n.contains("rounded") => "process",
        _ => return None,
    })
}

/// Flatten shapes into nodes and groups, in page pixels with y down.
fn build(shapes: &[Shape], connects: &[Connect], masters: &HashMap<String, String>, out: &mut Out) {
    // Absolute boxes first: (shape, left, bottom) in inches.
    let mut flat: Vec<(&Shape, f64, f64, Option<String>)> = Vec::new();
    fn walk<'a>(s: &'a Shape, origin: (f64, f64), parent: Option<String>, flat: &mut Vec<(&'a Shape, f64, f64, Option<String>)>) {
        let left = origin.0 + s.pin.0 - s.loc.0;
        let bottom = origin.1 + s.pin.1 - s.loc.1;
        flat.push((s, left, bottom, parent));
        if s.group {
            for c in &s.children {
                walk(c, (left, bottom), Some(s.id.clone()), flat);
            }
        }
    }
    for s in shapes {
        walk(s, (0.0, 0.0), None, &mut flat);
    }
    let top = flat.iter().filter(|(s, ..)| s.begin.is_none()).map(|(s, _, b, _)| b + s.size.1).fold(0.0, f64::max);
    let id_of = |s: &Shape| ident(&format!("s{}", s.id));
    let mut groups: HashMap<String, OGroup> = HashMap::new();
    let connectors: HashMap<&str, &Shape> = flat.iter().filter(|(s, ..)| s.begin.is_some() && s.end.is_some()).map(|(s, ..)| (s.id.as_str(), *s)).collect();
    for (s, left, bottom, parent) in &flat {
        if connectors.contains_key(s.id.as_str()) {
            continue;
        }
        let geom = (left * PX, (top - bottom - s.size.1) * PX, s.size.0 * PX, s.size.1 * PX);
        let label = s.text.trim().to_string();
        if s.group {
            groups.insert(s.id.clone(), OGroup { id: id_of(s), label: (!label.is_empty()).then_some(label), members: Vec::new(), props: Vec::new(), geom: Some(geom) });
        } else {
            let master = s.master.as_ref().and_then(|m| masters.get(m)).cloned().unwrap_or_else(|| s.name.clone());
            let n = out.node(&id_of(s));
            n.stencil = stencil(&master);
            n.label = (!label.is_empty()).then_some(label);
            n.geom = Some(geom);
        }
        if let Some(p) = parent
            && let Some(g) = groups.get_mut(p)
        {
            g.members.push(id_of(s));
        }
    }
    // Groups come after their members are known.
    let mut gs: Vec<OGroup> = groups.into_values().filter(|g| !g.members.is_empty()).collect();
    gs.sort_by(|a, b| a.id.cmp(&b.id));
    out.groups.extend(gs);
    // A connector's two glued ends name its shapes.
    let mut ends: HashMap<&str, (Option<&str>, Option<&str>)> = HashMap::new();
    for (from, cell, to) in connects {
        let e = ends.entry(from.as_str()).or_default();
        match cell.as_str() {
            "BeginX" => e.0 = Some(to),
            "EndX" => e.1 = Some(to),
            _ => {}
        }
    }
    let mut ids: Vec<&&str> = connectors.keys().collect();
    ids.sort_by_key(|id| id.parse::<u32>().unwrap_or(u32::MAX));
    for id in ids {
        let c = connectors[*id];
        match ends.get(*id) {
            Some((Some(a), Some(b))) => out.edges.push(OEdge {
                from: ident(&format!("s{a}")),
                to: ident(&format!("s{b}")),
                arrow: "->",
                label: Some(c.text.trim().to_string()).filter(|t| !t.is_empty()),
                props: Vec::new(),
                via: Vec::new(),
            }),
            _ => out.warnings.push(format!("connector {id} is not glued at both ends; skipped")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    /// A two-shape page with one connector, zipped like Visio does.
    pub(crate) fn sample() -> Vec<u8> {
        let page = r#"<?xml version="1.0" encoding="utf-8"?>
<PageContents xmlns="http://schemas.microsoft.com/office/visio/2012/main">
  <Shapes>
    <Shape ID="1" NameU="Start" Master="1" Type="Shape">
      <Cell N="PinX" V="2"/><Cell N="PinY" V="9"/><Cell N="Width" V="1.5"/><Cell N="Height" V="0.75"/>
      <Cell N="LocPinX" V="0.75"/><Cell N="LocPinY" V="0.375"/>
      <Text>Start &amp; go</Text>
    </Shape>
    <Shape ID="2" NameU="Decision" Master="2" Type="Shape">
      <Cell N="PinX" V="2"/><Cell N="PinY" V="7"/><Cell N="Width" V="1.5"/><Cell N="Height" V="1"/>
      <Cell N="LocPinX" V="0.75"/><Cell N="LocPinY" V="0.5"/>
      <Text>OK?</Text>
    </Shape>
    <Shape ID="3" NameU="Dynamic connector" Master="3" Type="Shape">
      <Cell N="BeginX" V="2"/><Cell N="BeginY" V="8.6"/><Cell N="EndX" V="2"/><Cell N="EndY" V="7.5"/>
      <Text>next</Text>
    </Shape>
  </Shapes>
  <Connects>
    <Connect FromSheet="3" FromCell="BeginX" ToSheet="1"/>
    <Connect FromSheet="3" FromCell="EndX" ToSheet="2"/>
  </Connects>
</PageContents>"#;
        let masters = r#"<Masters><Master ID="1" NameU="Start/End"/><Master ID="2" NameU="Decision"/><Master ID="3" NameU="Dynamic connector"/></Masters>"#;
        let mut buf = Vec::new();
        {
            let mut z = zip::ZipWriter::new(Cursor::new(&mut buf));
            let opts = zip::write::SimpleFileOptions::default();
            z.start_file("visio/pages/page1.xml", opts).unwrap();
            z.write_all(page.as_bytes()).unwrap();
            z.start_file("visio/masters/masters.xml", opts).unwrap();
            z.write_all(masters.as_bytes()).unwrap();
            z.finish().unwrap();
        }
        buf
    }

    #[test]
    fn shapes_and_connectors_come_across() {
        let out = parse(&sample()).unwrap();
        assert_eq!(out.nodes.len(), 2);
        assert_eq!(out.nodes[0].stencil, Some("terminal"));
        assert_eq!(out.nodes[0].label.as_deref(), Some("Start & go"));
        assert_eq!(out.nodes[1].stencil, Some("decision"));
        // y flips: Start sits above the decision.
        assert!(out.nodes[0].geom.unwrap().1 < out.nodes[1].geom.unwrap().1);
        assert_eq!(out.nodes[0].geom.unwrap().2, 144.0);
        assert_eq!(out.edges.len(), 1);
        assert_eq!((out.edges[0].from.as_str(), out.edges[0].to.as_str(), out.edges[0].label.as_deref()), ("s1", "s2", Some("next")));
    }

    #[test]
    fn a_picked_format_wins_over_the_extension() {
        let dir = std::env::temp_dir().join(format!("graphing-import-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A Visio file with a misleading name still imports as Visio.
        let odd = dir.join("drawing.bin");
        std::fs::write(&odd, sample()).unwrap();
        let out = crate::import_file_as(&odd, Some(crate::Format::Visio)).unwrap();
        assert!(out.source.contains("s1: terminal"), "{}", out.source);
        // And without a pick, its zip header gives it away.
        assert!(crate::import_file(&odd).unwrap().source.contains("s2: decision"));
        // Mermaid text picked as mermaid, whatever the extension.
        let text = dir.join("notes.txt");
        std::fs::write(&text, "flowchart LR\n  a --> b\n").unwrap();
        assert!(crate::import_file_as(&text, Some(crate::Format::Mermaid)).unwrap().source.contains("a -> b"));
        assert_eq!(crate::Format::parse("visio"), Some(crate::Format::Visio));
    }
}
