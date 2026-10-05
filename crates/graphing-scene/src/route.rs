//! Orthogonal (elbow) routing: lines that run horizontal and vertical,
//! around shapes rather than through them.
//!
//! One grid serves every line of a scene: the lines through each shape's
//! sides (pushed out by a margin) and centre. A grid point inside a shape's
//! margin is blocked; so is a grid segment whose middle is, which is exact
//! because every shape edge is a grid line. Each route is an A* search over
//! (point, heading) with a penalty per bend, from the four side stubs of
//! its source to the four of its target.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use graphing_model::{Point, Rect};

/// Room kept between a line and the shapes it passes.
pub const MARGIN: f64 = 18.0;
/// What a bend costs, in units of length: fewer bends beat shorter lines.
const BEND: f64 = 60.0;
/// What sharing a grid segment with an earlier line costs, so lines spread
/// out instead of running on top of each other.
const SHARED: f64 = 45.0;

/// Headings: right, down, left, up.
const DIRS: [(i32, i32); 4] = [(1, 0), (0, 1), (-1, 0), (0, -1)];

pub struct Grid {
    xs: Vec<f64>,
    ys: Vec<f64>,
    /// Obstacles, already grown by the margin.
    blocks: Vec<Rect>,
    /// Shape id -> its rect (without margin).
    shapes: HashMap<String, Rect>,
    /// Cached segment checks: (from point, heading) -> open.
    open: Vec<[bool; 4]>,
    /// How many routed lines already use each segment.
    used: Vec<[u8; 4]>,
}

fn inside(r: &Rect, p: Point) -> bool {
    p.x > r.origin.x + 1e-6 && p.x < r.origin.x + r.size.w - 1e-6 && p.y > r.origin.y + 1e-6 && p.y < r.origin.y + r.size.h - 1e-6
}

/// A side's stub: where a line leaves or enters it, and the outward heading.
fn stubs(r: Rect) -> [(Point, Point, usize); 4] {
    let c = r.center();
    let (l, t, rt, b) = (r.origin.x, r.origin.y, r.origin.x + r.size.w, r.origin.y + r.size.h);
    [
        (Point::new(rt, c.y), Point::new(rt + MARGIN, c.y), 0),
        (Point::new(c.x, b), Point::new(c.x, b + MARGIN), 1),
        (Point::new(l, c.y), Point::new(l - MARGIN, c.y), 2),
        (Point::new(c.x, t), Point::new(c.x, t - MARGIN), 3),
    ]
}

#[derive(PartialEq)]
struct Open {
    cost: f64,
    state: usize,
}
impl Eq for Open {}
impl Ord for Open {
    fn cmp(&self, o: &Self) -> Ordering {
        o.cost.total_cmp(&self.cost)
    }
}
impl PartialOrd for Open {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl Grid {
    /// The grid around `shapes` (id, rect).
    pub fn new(shapes: &[(String, Rect)]) -> Self {
        let mut xs = Vec::new();
        let mut ys = Vec::new();
        let blocks: Vec<Rect> = shapes.iter().map(|(_, r)| r.inflate(MARGIN)).collect();
        for (_, r) in shapes {
            let g = r.inflate(MARGIN);
            xs.extend([g.origin.x, g.origin.x + g.size.w, r.center().x]);
            ys.extend([g.origin.y, g.origin.y + g.size.h, r.center().y]);
        }
        // Channels down the middle of gaps read better than hugging a side.
        let mid = |v: &mut Vec<f64>| {
            v.sort_by(f64::total_cmp);
            v.dedup_by(|a, b| (*a - *b).abs() < 0.5);
            let extra: Vec<f64> = v.windows(2).filter(|w| w[1] - w[0] > MARGIN * 3.0).map(|w| (w[0] + w[1]) / 2.0).collect();
            v.extend(extra);
            v.sort_by(f64::total_cmp);
        };
        mid(&mut xs);
        mid(&mut ys);
        let mut grid = Grid { xs, ys, blocks, shapes: shapes.iter().cloned().collect(), open: Vec::new(), used: Vec::new() };
        let (nx, ny) = (grid.xs.len(), grid.ys.len());
        grid.open = vec![[false; 4]; nx * ny];
        grid.used = vec![[0; 4]; nx * ny];
        for j in 0..ny {
            for i in 0..nx {
                let p = grid.at(i, j);
                if grid.blocked(p) {
                    continue;
                }
                for (d, (dx, dy)) in DIRS.iter().enumerate() {
                    let (ni, nj) = (i as i32 + dx, j as i32 + dy);
                    if ni < 0 || nj < 0 || ni as usize >= nx || nj as usize >= ny {
                        continue;
                    }
                    let q = grid.at(ni as usize, nj as usize);
                    let m = Point::new((p.x + q.x) / 2.0, (p.y + q.y) / 2.0);
                    grid.open[j * nx + i][d] = !grid.blocked(q) && !grid.blocked(m);
                }
            }
        }
        grid
    }

    fn at(&self, i: usize, j: usize) -> Point {
        Point::new(self.xs[i], self.ys[j])
    }

    fn blocked(&self, p: Point) -> bool {
        self.blocks.iter().any(|b| inside(b, p))
    }

    fn index(&self, p: Point) -> Option<usize> {
        let i = self.xs.iter().position(|x| (x - p.x).abs() < 0.5)?;
        let j = self.ys.iter().position(|y| (y - p.y).abs() < 0.5)?;
        Some(j * self.xs.len() + i)
    }

    /// An elbow route from shape `from` to shape `to`, ends on their sides;
    /// `None` when either is unknown or no way round exists.
    pub fn route(&mut self, from: &str, to: &str) -> Option<Vec<Point>> {
        let (a, b) = (*self.shapes.get(from)?, *self.shapes.get(to)?);
        if from == to {
            // A loop off the right side, back into the top.
            let (s, t) = (stubs(a)[0], stubs(a)[3]);
            let corner = Point::new(s.1.x + MARGIN, t.1.y - MARGIN);
            return Some(vec![s.0, Point::new(corner.x, s.0.y), corner, Point::new(t.0.x, corner.y), t.0]);
        }
        let nx = self.xs.len();
        let n = self.open.len();
        let mut best = vec![f64::INFINITY; n * 4];
        let mut prev = vec![usize::MAX; n * 4];
        let mut heap = BinaryHeap::new();
        let goal_c = b.center();
        let h = |v: usize| {
            let p = self.at(v % nx, v / nx);
            (p.x - goal_c.x).abs() + (p.y - goal_c.y).abs()
        };
        for (port, stub, d) in stubs(a) {
            let Some(v) = self.index(stub) else { continue };
            let s = v * 4 + d;
            let cost = (stub.x - port.x).abs() + (stub.y - port.y).abs();
            if cost < best[s] {
                best[s] = cost;
                heap.push(Open { cost: cost + h(v), state: s });
            }
        }
        // Arriving at a target stub heading into the shape.
        let goals: HashMap<usize, (Point, usize)> = stubs(b).iter().filter_map(|&(port, stub, d)| Some((self.index(stub)?, (port, (d + 2) % 4)))).collect();
        let mut found: Option<(usize, f64)> = None;
        while let Some(Open { state, .. }) = heap.pop() {
            let (v, d) = (state / 4, state % 4);
            let g = best[state];
            if found.is_some_and(|(_, c)| g >= c) {
                break;
            }
            if let Some(&(_, inward)) = goals.get(&v) {
                let total = g + if d == inward { 0.0 } else { BEND };
                if found.is_none_or(|(_, c)| total < c) {
                    found = Some((state, total));
                }
            }
            let p = self.at(v % nx, v / nx);
            for (nd, (dx, dy)) in DIRS.iter().enumerate() {
                // No U-turns.
                if nd == (d + 2) % 4 || !self.open[v][nd] {
                    continue;
                }
                let (ni, nj) = ((v % nx) as i32 + dx, (v / nx) as i32 + dy);
                let w = nj as usize * nx + ni as usize;
                let q = self.at(ni as usize, nj as usize);
                let shared = self.used[v][nd] as f64 * SHARED;
                let cost = g + (q.x - p.x).abs() + (q.y - p.y).abs() + shared + if nd == d { 0.0 } else { BEND };
                let s = w * 4 + nd;
                if cost < best[s] {
                    best[s] = cost;
                    prev[s] = state;
                    heap.push(Open { cost: cost + h(w), state: s });
                }
            }
        }
        let (mut s, _) = found?;
        let end_port = goals[&(s / 4)].0;
        let mut pts = vec![end_port];
        loop {
            let v = s / 4;
            pts.push(self.at(v % nx, v / nx));
            if prev[s] == usize::MAX {
                break;
            }
            // Mark the step taken, both ways, for the lines still to come.
            let (d, u) = (s % 4, prev[s] / 4);
            self.used[u][d] = self.used[u][d].saturating_add(1);
            self.used[v][(d + 2) % 4] = self.used[v][(d + 2) % 4].saturating_add(1);
            s = prev[s];
        }
        // The first stub: back to its side.
        let first = *pts.last().expect("start");
        let port = stubs(a).into_iter().find(|(_, stub, _)| (stub.x - first.x).abs() < 0.5 && (stub.y - first.y).abs() < 0.5).map(|x| x.0)?;
        pts.push(port);
        pts.reverse();
        Some(simplify(pts))
    }
}

/// Drop points in the middle of straight runs.
fn simplify(pts: Vec<Point>) -> Vec<Point> {
    let mut out: Vec<Point> = Vec::with_capacity(pts.len());
    for p in pts {
        if out.last().is_some_and(|q| (q.x - p.x).abs() < 1e-6 && (q.y - p.y).abs() < 1e-6) {
            continue;
        }
        if out.len() >= 2 {
            let (a, b) = (out[out.len() - 2], out[out.len() - 1]);
            let straight = ((a.x - b.x).abs() < 1e-6 && (b.x - p.x).abs() < 1e-6) || ((a.y - b.y).abs() < 1e-6 && (b.y - p.y).abs() < 1e-6);
            if straight {
                out.pop();
            }
        }
        out.push(p);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn axis_aligned(pts: &[Point]) -> bool {
        pts.windows(2).all(|w| (w[0].x - w[1].x).abs() < 1e-6 || (w[0].y - w[1].y).abs() < 1e-6)
    }

    #[test]
    fn side_by_side_is_one_straight_run() {
        let mut g = Grid::new(&[("a".into(), Rect::new(0.0, 0.0, 100.0, 50.0)), ("b".into(), Rect::new(300.0, 0.0, 100.0, 50.0))]);
        let r = g.route("a", "b").unwrap();
        assert_eq!(r, [Point::new(100.0, 25.0), Point::new(300.0, 25.0)]);
    }

    #[test]
    fn routes_bend_round_what_is_in_the_way() {
        let shapes = [
            ("a".to_string(), Rect::new(0.0, 0.0, 100.0, 50.0)),
            ("wall".to_string(), Rect::new(180.0, -60.0, 60.0, 170.0)),
            ("b".to_string(), Rect::new(320.0, 0.0, 100.0, 50.0)),
        ];
        let mut g = Grid::new(&shapes);
        let r = g.route("a", "b").unwrap();
        assert!(axis_aligned(&r), "{r:?}");
        assert!(r.len() > 2, "goes round: {r:?}");
        // No segment passes through the wall.
        let wall = shapes[1].1;
        for w in r.windows(2) {
            let m = Point::new((w[0].x + w[1].x) / 2.0, (w[0].y + w[1].y) / 2.0);
            assert!(!inside(&wall, m), "{r:?}");
        }
        // It starts on a's border and ends on b's.
        let on = |r: Rect, p: Point| {
            let (l, t, rt, b) = (r.origin.x, r.origin.y, r.origin.x + r.size.w, r.origin.y + r.size.h);
            ((p.x - l).abs() < 1e-6 || (p.x - rt).abs() < 1e-6) && p.y >= t && p.y <= b || ((p.y - t).abs() < 1e-6 || (p.y - b).abs() < 1e-6) && p.x >= l && p.x <= rt
        };
        assert!(on(shapes[0].1, r[0]) && on(shapes[2].1, *r.last().unwrap()), "{r:?}");
    }

    #[test]
    fn diagonal_neighbours_get_one_elbow() {
        let mut g = Grid::new(&[("a".into(), Rect::new(0.0, 0.0, 100.0, 50.0)), ("b".into(), Rect::new(300.0, 200.0, 100.0, 50.0))]);
        let r = g.route("a", "b").unwrap();
        assert!(axis_aligned(&r), "{r:?}");
        assert!(r.len() <= 4, "{r:?}");
    }

    #[test]
    fn a_second_line_finds_its_own_way() {
        let mut g = Grid::new(&[("a".into(), Rect::new(0.0, 0.0, 100.0, 50.0)), ("b".into(), Rect::new(300.0, 200.0, 100.0, 50.0))]);
        let first = g.route("a", "b").unwrap();
        let second = g.route("a", "b").unwrap();
        assert_ne!(first, second);
    }
}
