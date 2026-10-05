//! SVG path data for custom stencil outlines, parsed once into absolute
//! commands in the stencil's own coordinate box (usually 0..1 or the
//! `viewBox` the pack declares) and scaled into each node's rect.

use graphing_model::{Point, Rect};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathCmd {
    Move(Point),
    Line(Point),
    Cubic(Point, Point, Point),
    Quad(Point, Point),
    Arc { rx: f64, ry: f64, rotation: f64, large: bool, sweep: bool, to: Point },
    Close,
}

/// Parsed outline plus the box its coordinates live in.
#[derive(Debug, Clone, PartialEq)]
pub struct Outline {
    pub cmds: Vec<PathCmd>,
    pub view: Rect,
}

impl Outline {
    /// Commands mapped from the outline's box into `r`.
    pub fn fit(&self, r: Rect) -> Vec<PathCmd> {
        let sx = r.size.w / self.view.size.w.max(f64::EPSILON);
        let sy = r.size.h / self.view.size.h.max(f64::EPSILON);
        let map = |p: Point| Point::new(r.origin.x + (p.x - self.view.origin.x) * sx, r.origin.y + (p.y - self.view.origin.y) * sy);
        self.cmds
            .iter()
            .map(|c| match *c {
                PathCmd::Move(p) => PathCmd::Move(map(p)),
                PathCmd::Line(p) => PathCmd::Line(map(p)),
                PathCmd::Cubic(a, b, p) => PathCmd::Cubic(map(a), map(b), map(p)),
                PathCmd::Quad(a, p) => PathCmd::Quad(map(a), map(p)),
                PathCmd::Arc { rx, ry, rotation, large, sweep, to } => PathCmd::Arc { rx: rx * sx, ry: ry * sy, rotation, large, sweep, to: map(to) },
                PathCmd::Close => PathCmd::Close,
            })
            .collect()
    }
}

/// Parse SVG path data (`M L H V C S Q T A Z`, absolute and relative).
pub fn parse(d: &str) -> Result<Vec<PathCmd>, String> {
    let mut t = Tokens { s: d.as_bytes(), i: 0 };
    let mut out = Vec::new();
    let (mut cur, mut start) = (Point::default(), Point::default());
    // Last control point, for S and T reflection.
    let mut last_cubic: Option<Point> = None;
    let mut last_quad: Option<Point> = None;
    let mut cmd = b'M';
    loop {
        t.skip_sep();
        if t.i >= t.s.len() {
            break;
        }
        let c = t.s[t.i];
        if c.is_ascii_alphabetic() {
            cmd = c;
            t.i += 1;
        } else if cmd == b'Z' || cmd == b'z' {
            return Err(format!("number after Z at {}", t.i));
        }
        let rel = cmd.is_ascii_lowercase();
        let at = |p: Point, cur: Point| if rel { Point::new(cur.x + p.x, cur.y + p.y) } else { p };
        let upper = cmd.to_ascii_uppercase();
        match upper {
            b'M' => {
                let p = at(t.point()?, cur);
                out.push(PathCmd::Move(p));
                cur = p;
                start = p;
                // Further pairs after M are implicit L.
                cmd = if rel { b'l' } else { b'L' };
            }
            b'L' => {
                cur = at(t.point()?, cur);
                out.push(PathCmd::Line(cur));
            }
            b'H' => {
                let x = t.num()?;
                cur = Point::new(if rel { cur.x + x } else { x }, cur.y);
                out.push(PathCmd::Line(cur));
            }
            b'V' => {
                let y = t.num()?;
                cur = Point::new(cur.x, if rel { cur.y + y } else { y });
                out.push(PathCmd::Line(cur));
            }
            b'C' => {
                let a = at(t.point()?, cur);
                let b = at(t.point()?, cur);
                let p = at(t.point()?, cur);
                out.push(PathCmd::Cubic(a, b, p));
                last_cubic = Some(b);
                cur = p;
            }
            b'S' => {
                let a = last_cubic.map_or(cur, |c| Point::new(2.0 * cur.x - c.x, 2.0 * cur.y - c.y));
                let b = at(t.point()?, cur);
                let p = at(t.point()?, cur);
                out.push(PathCmd::Cubic(a, b, p));
                last_cubic = Some(b);
                cur = p;
            }
            b'Q' => {
                let a = at(t.point()?, cur);
                let p = at(t.point()?, cur);
                out.push(PathCmd::Quad(a, p));
                last_quad = Some(a);
                cur = p;
            }
            b'T' => {
                let a = last_quad.map_or(cur, |c| Point::new(2.0 * cur.x - c.x, 2.0 * cur.y - c.y));
                let p = at(t.point()?, cur);
                out.push(PathCmd::Quad(a, p));
                last_quad = Some(a);
                cur = p;
            }
            b'A' => {
                let rx = t.num()?.abs();
                let ry = t.num()?.abs();
                let rotation = t.num()?;
                let large = t.flag()?;
                let sweep = t.flag()?;
                let to = at(t.point()?, cur);
                out.push(PathCmd::Arc { rx, ry, rotation, large, sweep, to });
                cur = to;
            }
            b'Z' => {
                out.push(PathCmd::Close);
                cur = start;
            }
            other => return Err(format!("unknown path command `{}`", other as char)),
        }
        if !matches!(upper, b'C' | b'S') {
            last_cubic = None;
        }
        if !matches!(upper, b'Q' | b'T') {
            last_quad = None;
        }
    }
    if out.is_empty() {
        return Err("empty path".into());
    }
    Ok(out)
}

struct Tokens<'a> {
    s: &'a [u8],
    i: usize,
}

impl Tokens<'_> {
    fn skip_sep(&mut self) {
        while self.i < self.s.len() && (self.s[self.i].is_ascii_whitespace() || self.s[self.i] == b',') {
            self.i += 1;
        }
    }

    fn num(&mut self) -> Result<f64, String> {
        self.skip_sep();
        let start = self.i;
        let s = self.s;
        if self.i < s.len() && (s[self.i] == b'-' || s[self.i] == b'+') {
            self.i += 1;
        }
        let mut dot = false;
        while self.i < s.len() && (s[self.i].is_ascii_digit() || (s[self.i] == b'.' && !dot)) {
            dot |= s[self.i] == b'.';
            self.i += 1;
        }
        if self.i < s.len() && (s[self.i] == b'e' || s[self.i] == b'E') {
            self.i += 1;
            if self.i < s.len() && (s[self.i] == b'-' || s[self.i] == b'+') {
                self.i += 1;
            }
            while self.i < s.len() && s[self.i].is_ascii_digit() {
                self.i += 1;
            }
        }
        std::str::from_utf8(&s[start..self.i]).ok().and_then(|t| t.parse().ok()).ok_or_else(|| format!("expected a number at {start}"))
    }

    fn point(&mut self) -> Result<Point, String> {
        Ok(Point::new(self.num()?, self.num()?))
    }

    /// Arc flags may be written without separators (`a1 1 0 01 1 1`).
    fn flag(&mut self) -> Result<bool, String> {
        self.skip_sep();
        match self.s.get(self.i) {
            Some(b'0') => {
                self.i += 1;
                Ok(false)
            }
            Some(b'1') => {
                self.i += 1;
                Ok(true)
            }
            _ => Err(format!("expected an arc flag at {}", self.i)),
        }
    }
}

/// Points along the outline (curves sampled), for edge clipping.
pub fn polyline(cmds: &[PathCmd]) -> Vec<Point> {
    let mut pts = Vec::new();
    let mut cur = Point::default();
    for c in cmds {
        match *c {
            PathCmd::Move(p) | PathCmd::Line(p) => {
                pts.push(p);
                cur = p;
            }
            PathCmd::Cubic(a, b, p) => {
                for i in 1..=8 {
                    let t = i as f64 / 8.0;
                    let u = 1.0 - t;
                    pts.push(Point::new(
                        u * u * u * cur.x + 3.0 * u * u * t * a.x + 3.0 * u * t * t * b.x + t * t * t * p.x,
                        u * u * u * cur.y + 3.0 * u * u * t * a.y + 3.0 * u * t * t * b.y + t * t * t * p.y,
                    ));
                }
                cur = p;
            }
            PathCmd::Quad(a, p) => {
                for i in 1..=6 {
                    let t = i as f64 / 6.0;
                    let u = 1.0 - t;
                    pts.push(Point::new(u * u * cur.x + 2.0 * u * t * a.x + t * t * p.x, u * u * cur.y + 2.0 * u * t * a.y + t * t * p.y));
                }
                cur = p;
            }
            PathCmd::Arc { to, .. } => {
                pts.push(to);
                cur = to;
            }
            PathCmd::Close => {}
        }
    }
    pts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_forms() {
        let c = parse("M0 0 H1 V1 h-1 z").unwrap();
        assert_eq!(c.len(), 5);
        assert_eq!(c[3], PathCmd::Line(Point::new(0.0, 1.0)));
        let c = parse("m10,10 l5-5 5,5 C 1 2 3 4 5 6 S 7 8 9 10 a5 5 0 01 10 0Z").unwrap();
        assert!(matches!(c[1], PathCmd::Line(p) if p == Point::new(15.0, 5.0)));
        assert!(matches!(c[2], PathCmd::Line(p) if p == Point::new(20.0, 10.0)));
        // S reflects the previous control point (3,4) about (5,6).
        assert!(matches!(c[4], PathCmd::Cubic(a, _, _) if a == Point::new(7.0, 8.0)));
        assert!(matches!(c[5], PathCmd::Arc { large: false, sweep: true, .. }));
        assert!(parse("M0 0 X1").is_err());
        assert!(parse("").is_err());
    }

    #[test]
    fn fits_into_rect() {
        let o = Outline { cmds: parse("M0 0 L1 1").unwrap(), view: Rect::new(0.0, 0.0, 1.0, 1.0) };
        let f = o.fit(Rect::new(10.0, 20.0, 100.0, 50.0));
        assert_eq!(f[1], PathCmd::Line(Point::new(110.0, 70.0)));
    }
}
