//! draw.io / diagrams.net (mxGraph XML), plain or compressed pages.

use std::collections::HashMap;
use std::io::Read as _;

use base64::Engine as _;
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use crate::{ImportError, OEdge, OGroup, ONode, Out, ident, value};

#[derive(Debug, Default, Clone)]
struct Cell {
    id: String,
    value: String,
    style: HashMap<String, String>,
    vertex: bool,
    edge: bool,
    parent: String,
    source: String,
    target: String,
    geom: Option<(f64, f64, f64, f64)>,
    points: Vec<(f64, f64)>,
}

pub(crate) fn parse(src: &str) -> Result<Out, ImportError> {
    let mut out = Out::default();
    let model = graph_model(src, &mut out.warnings)?;
    let cells = cells(&model)?;
    build(cells, &mut out);
    Ok(out)
}

/// XML of the first page's `<mxGraphModel>`, decompressing if needed.
fn graph_model(src: &str, warnings: &mut Vec<String>) -> Result<String, ImportError> {
    if !src.contains("<mxfile") {
        return Ok(src.to_string());
    }
    let mut reader = Reader::from_str(src);
    let mut pages = 0;
    let mut first: Option<String> = None;
    let mut in_diagram = false;
    let mut start = 0usize;
    loop {
        let pos = reader.buffer_position() as usize;
        match reader.read_event() {
            Ok(Event::Start(e)) if e.name().as_ref() == "diagram" => {
                pages += 1;
                in_diagram = true;
                start = reader.buffer_position() as usize;
                if let Some(name) = attr(&e, "name")
                    && pages == 1
                {
                    let _ = name;
                }
            }
            Ok(Event::End(e)) if e.name().as_ref() == "diagram" => {
                if pages == 1 {
                    first = Some(src[start..pos].to_string());
                }
                in_diagram = false;
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(ImportError::Parse(format!("draw.io xml: {e}"))),
            _ => {}
        }
        let _ = in_diagram;
    }
    if pages > 1 {
        warnings.push(format!("{pages} pages; imported the first"));
    }
    let body = first.ok_or_else(|| ImportError::Parse("no <diagram> page".into()))?;
    let body = body.trim();
    if body.contains("<mxGraphModel") {
        return Ok(body.to_string());
    }
    // base64 -> raw deflate -> URI-encoded XML
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(body.split_whitespace().collect::<String>())
        .map_err(|e| ImportError::Parse(format!("page is not base64: {e}")))?;
    let mut inflated = String::new();
    flate2::read::DeflateDecoder::new(&bytes[..])
        .read_to_string(&mut inflated)
        .map_err(|e| ImportError::Parse(format!("page does not inflate: {e}")))?;
    Ok(percent_decode(&inflated))
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn attr(e: &BytesStart, name: &str) -> Option<String> {
    e.attributes().flatten().find(|a| a.key.as_ref() == name).and_then(|a| a.normalized_value(quick_xml::XmlVersion::default()).ok()).map(|v| v.into_owned())
}

fn num(e: &BytesStart, name: &str) -> f64 {
    attr(e, name).and_then(|v| v.parse().ok()).unwrap_or(0.0)
}

fn style_map(s: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for p in s.split(';').filter(|p| !p.is_empty()) {
        match p.split_once('=') {
            Some((k, v)) => {
                map.insert(k.to_string(), v.to_string());
            }
            None => {
                // Bare tokens (`ellipse`, `swimlane`, `edgeLabel`) are flags
                // and, first one wins, the shape name.
                map.insert(p.to_string(), String::new());
                map.entry("shape".to_string()).or_insert_with(|| p.to_string());
            }
        }
    }
    map
}

fn cells(xml: &str) -> Result<Vec<Cell>, ImportError> {
    let mut reader = Reader::from_str(xml);
    let mut cells = Vec::new();
    let mut current: Option<Cell> = None;
    let mut in_points = false;
    loop {
        let ev = reader.read_event().map_err(|e| ImportError::Parse(format!("draw.io xml: {e}")))?;
        let (e, empty) = match &ev {
            Event::Start(e) => (e.clone(), false),
            Event::Empty(e) => (e.clone(), true),
            Event::End(e) => {
                match e.name().as_ref() {
                    "mxCell" => cells.extend(current.take()),
                    "Array" => in_points = false,
                    _ => {}
                }
                continue;
            }
            Event::Eof => break,
            _ => continue,
        };
        match e.name().as_ref() {
            "mxCell" => {
                let cell = Cell {
                    id: attr(&e, "id").unwrap_or_default(),
                    value: attr(&e, "value").unwrap_or_default(),
                    style: style_map(&attr(&e, "style").unwrap_or_default()),
                    vertex: attr(&e, "vertex").as_deref() == Some("1"),
                    edge: attr(&e, "edge").as_deref() == Some("1"),
                    parent: attr(&e, "parent").unwrap_or_default(),
                    source: attr(&e, "source").unwrap_or_default(),
                    target: attr(&e, "target").unwrap_or_default(),
                    ..Default::default()
                };
                if empty {
                    cells.push(cell);
                } else {
                    current = Some(cell);
                }
            }
            "mxGeometry" => {
                if let Some(c) = current.as_mut()
                    && attr(&e, "relative").as_deref() != Some("1")
                {
                    c.geom = Some((num(&e, "x"), num(&e, "y"), num(&e, "width"), num(&e, "height")));
                }
            }
            "Array" if attr(&e, "as").as_deref() == Some("points") => in_points = !empty,
            "mxPoint" if in_points => {
                if let Some(c) = current.as_mut() {
                    c.points.push((num(&e, "x"), num(&e, "y")));
                }
            }
            _ => {}
        }
    }
    Ok(cells)
}

/// HTML label -> plain text with newlines.
fn plain(html: &str) -> String {
    let mut s = html.replace("<br>", "\n").replace("<br/>", "\n").replace("<br />", "\n").replace("<div>", "\n");
    while let Some(a) = s.find('<') {
        let Some(b) = s[a..].find('>') else { break };
        s.replace_range(a..a + b + 1, "");
    }
    let s = s.replace("&nbsp;", " ").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&amp;", "&");
    s.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
}

fn stencil(style: &HashMap<String, String>) -> Option<&'static str> {
    let shape = style.get("shape").map(String::as_str).unwrap_or("");
    Some(match shape {
        "cylinder" | "cylinder3" | "datastore" => "db",
        "rhombus" => "decision",
        "ellipse" | "doubleEllipse" => "ellipse",
        "hexagon" => "hexagon",
        "parallelogram" => "io",
        "note" => "note",
        "umlActor" => "actor",
        _ if style.get("rounded").map(String::as_str) == Some("1") => "rounded",
        _ => return None,
    })
}

fn is_container(style: &HashMap<String, String>) -> bool {
    style.contains_key("swimlane")
        || style.get("shape").is_some_and(|s| s == "swimlane" || s == "group")
        || style.get("container").map(String::as_str) == Some("1")
        || style.contains_key("group")
}

fn color_props(style: &HashMap<String, String>, keys: &[(&str, &str)]) -> Vec<(String, String)> {
    keys.iter()
        .filter_map(|(from, to)| {
            let v = style.get(*from)?;
            (v != "none" && v != "default" && v.starts_with('#')).then(|| (to.to_string(), value(v)))
        })
        .collect()
}

/// Absolute offset of a cell's coordinate space (its parents' origins).
fn offset<'a>(by_id: &HashMap<&'a str, &'a Cell>, id: &'a str) -> (f64, f64) {
    let (mut x, mut y) = (0.0, 0.0);
    let mut cur = id;
    while let Some(c) = by_id.get(cur) {
        if let Some((px, py, _, _)) = c.geom
            && c.vertex
        {
            x += px;
            y += py;
        }
        cur = &c.parent;
    }
    (x, y)
}

fn build(cells: Vec<Cell>, out: &mut Out) {
    let by_id: HashMap<&str, &Cell> = cells.iter().map(|c| (c.id.as_str(), c)).collect();
    let offset = |id: &str| offset(&by_id, id);

    // Readable ids from labels.
    let mut names: HashMap<&str, String> = HashMap::new();
    let mut used: HashMap<String, usize> = HashMap::new();
    let mut auto = 0;
    for c in cells.iter().filter(|c| c.vertex) {
        let text = plain(&c.value);
        let base = if text.is_empty() {
            auto += 1;
            format!("n{auto}")
        } else {
            let slug: String = text.to_lowercase().chars().map(|ch| if ch.is_alphanumeric() { ch } else { '_' }).collect();
            let slug = slug.split('_').filter(|s| !s.is_empty()).collect::<Vec<_>>().join("_");
            ident(&slug.chars().take(24).collect::<String>())
        };
        let k = used.entry(base.clone()).or_insert(0);
        *k += 1;
        let name = if *k == 1 { base } else { format!("{base}_{k}") };
        names.insert(&c.id, name);
    }

    let containers: Vec<&Cell> = cells.iter().filter(|c| c.vertex && is_container(&c.style)).collect();
    let is_group = |id: &str| containers.iter().any(|g| g.id == id);
    // Edge label children (`edgeLabel` cells whose parent is an edge).
    let mut edge_labels: HashMap<&str, String> = HashMap::new();
    for c in &cells {
        if c.vertex
            && c.style.contains_key("edgeLabel")
            && by_id.get(c.parent.as_str()).is_some_and(|p| p.edge)
        {
            edge_labels.insert(&c.parent, plain(&c.value));
        }
    }

    for c in cells.iter().filter(|c| c.vertex && !c.style.contains_key("edgeLabel")) {
        let name = names[c.id.as_str()].clone();
        let (ox, oy) = offset(&c.parent);
        let geom = c.geom.map(|(x, y, w, h)| (x + ox, y + oy, w, h));
        if is_group(&c.id) {
            let label = plain(&c.value);
            out.groups.push(OGroup {
                id: name,
                label: (!label.is_empty()).then_some(label),
                members: Vec::new(),
                props: color_props(&c.style, &[("fillColor", "fill"), ("strokeColor", "stroke")]),
                geom,
            });
            continue;
        }
        let label = plain(&c.value);
        let node = ONode {
            id: name.clone(),
            stencil: stencil(&c.style),
            label: (!label.is_empty()).then_some(label),
            classes: Vec::new(),
            props: color_props(&c.style, &[("fillColor", "fill"), ("strokeColor", "stroke"), ("fontColor", "color")]),
            geom,
        };
        *out.node(&name) = node;
    }
    // Membership, now that every group exists.
    for c in cells.iter().filter(|c| c.vertex && is_group(&c.parent) && !c.style.contains_key("edgeLabel")) {
        let (child, parent) = (names[c.id.as_str()].clone(), names[c.parent.as_str()].clone());
        if let Some(g) = out.groups.iter_mut().find(|g| g.id == parent) {
            g.members.push(child);
        }
    }
    for c in cells.iter().filter(|c| c.edge) {
        let (Some(from), Some(to)) = (names.get(c.source.as_str()), names.get(c.target.as_str())) else {
            out.warnings.push(format!("edge `{}` has a missing end, skipped", c.id));
            continue;
        };
        let mut label = plain(&c.value);
        if label.is_empty()
            && let Some(l) = edge_labels.get(c.id.as_str())
        {
            label = l.clone();
        }
        let mut props = color_props(&c.style, &[("strokeColor", "stroke")]);
        if c.style.get("dashed").map(String::as_str) == Some("1") {
            props.push(("line".into(), "dashed".into()));
        }
        let start = c.style.get("startArrow").is_some_and(|a| a != "none");
        let end = c.style.get("endArrow").is_none_or(|a| a != "none");
        let arrow = match (start, end) {
            (true, true) => "<->",
            (false, true) => "->",
            (true, false) => "<-",
            (false, false) => "--",
        };
        let (ox, oy) = offset(&c.parent);
        out.edges.push(OEdge {
            from: from.clone(),
            to: to.clone(),
            arrow,
            label: (!label.is_empty()).then_some(label),
            props,
            via: c.points.iter().map(|(x, y)| (x + ox, y + oy)).collect(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{assert_clean, from_drawio};
    use graphing_model::{Arrow, Point};
    use std::io::Write as _;

    const MODEL: &str = r##"<mxGraphModel><root>
  <mxCell id="0"/>
  <mxCell id="1" parent="0"/>
  <mxCell id="a" value="Web &lt;b&gt;App&lt;/b&gt;" style="rounded=1;whiteSpace=wrap;fillColor=#dae8fc;strokeColor=#6c8ebf;" vertex="1" parent="1">
    <mxGeometry x="40" y="60" width="120" height="60" as="geometry"/>
  </mxCell>
  <mxCell id="g" value="Backend" style="swimlane;" vertex="1" parent="1">
    <mxGeometry x="300" y="20" width="300" height="200" as="geometry"/>
  </mxCell>
  <mxCell id="b" value="Postgres" style="shape=cylinder3;" vertex="1" parent="g">
    <mxGeometry x="20" y="40" width="80" height="80" as="geometry"/>
  </mxCell>
  <mxCell id="c" value="Ok?" style="rhombus;" vertex="1" parent="g">
    <mxGeometry x="150" y="40" width="80" height="80" as="geometry"/>
  </mxCell>
  <mxCell id="e1" value="query" style="dashed=1;" edge="1" parent="1" source="a" target="b">
    <mxGeometry relative="1" as="geometry">
      <Array as="points"><mxPoint x="200" y="90"/><mxPoint x="250" y="130"/></Array>
    </mxGeometry>
  </mxCell>
  <mxCell id="e2" style="startArrow=classic;" edge="1" parent="g" source="b" target="c">
    <mxGeometry relative="1" as="geometry"/>
  </mxCell>
  <mxCell id="e2l" value="check" style="edgeLabel;" vertex="1" connectable="0" parent="e2">
    <mxGeometry relative="1" as="geometry"/>
  </mxCell>
  <mxCell id="e3" edge="1" parent="1" source="a" target="zzz"/>
</root></mxGraphModel>"##;

    #[test]
    fn plain_model() {
        let out = from_drawio(MODEL).unwrap();
        let d = assert_clean(&out.source);
        let web = d.node("web_app").unwrap();
        assert_eq!(web.label.as_deref(), Some("Web App"));
        assert_eq!(web.stencil.as_deref(), Some("rounded"));
        assert_eq!(d.layout["web_app"].pos, Point::new(40.0, 60.0));
        // Child geometry is relative to the swimlane.
        assert_eq!(d.layout["postgres"].pos, Point::new(320.0, 60.0));
        assert_eq!(d.node("postgres").unwrap().stencil.as_deref(), Some("db"));
        assert_eq!(d.node("ok").unwrap().stencil.as_deref(), Some("decision"));
        let g = d.group("backend").unwrap();
        assert_eq!(g.members, ["postgres", "ok"]);
        let e = d.edge("web_app->postgres").unwrap();
        assert_eq!(e.label.as_deref(), Some("query"));
        assert!(d.edge_prop(e, "line").is_some());
        assert_eq!(d.waypoints["web_app->postgres"], [Point::new(200.0, 90.0), Point::new(250.0, 130.0)]);
        let e2 = d.edge("postgres->ok").unwrap();
        assert_eq!(e2.arrow, Arrow::Both);
        assert_eq!(e2.label.as_deref(), Some("check"));
        assert_eq!(out.warnings.len(), 1, "{:?}", out.warnings);
    }

    #[test]
    fn compressed_page() {
        let encoded: String = MODEL
            .bytes()
            .map(|b| if b.is_ascii_alphanumeric() { (b as char).to_string() } else { format!("%{b:02X}") })
            .collect();
        let mut z = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(encoded.as_bytes()).unwrap();
        let b64 = base64::engine::general_purpose::STANDARD.encode(z.finish().unwrap());
        let file = format!(r#"<mxfile host="app"><diagram id="p1" name="Page-1">{b64}</diagram><diagram id="p2" name="Two"><mxGraphModel><root/></mxGraphModel></diagram></mxfile>"#);
        let out = from_drawio(&file).unwrap();
        let d = assert_clean(&out.source);
        assert_eq!(d.nodes.len(), 3);
        assert!(out.warnings.iter().any(|w| w.contains("2 pages")));
    }
}
