//! Timing diagram waveforms from WaveDrom-style strings: one character per
//! time slot.
//!
//! | Char | Slot |
//! | --- | --- |
//! | `0` `1` (`l` `h` `L` `H` `d` `u`) | low or high level |
//! | `p` `P` / `n` `N` | a clock period, rising / falling first |
//! | `x` | unknown |
//! | `z` | high impedance (the middle) |
//! | `=` `2`..`9` | a data value, labelled from `data` in order |
//! | `.` | the slot before continues |
//! | `|` | a break in time; drawn as the slot before |
//!
//! The geometry is computed here so the canvas and the exporters draw the
//! same lines.

use graphing_model::{Point, Rect};

/// One piece of a waveform.
#[derive(Debug, Clone, PartialEq)]
pub enum WavePrim {
    /// A digital level line (levels, clocks, high impedance).
    Line(Vec<Point>),
    /// A bus or unknown span: a closed hexagon, with its label.
    Bus { points: Vec<Point>, label: Option<String>, unknown: bool, center: Point },
}

/// A laid-out signal: the label's column and the wave's pieces.
#[derive(Debug, Clone, PartialEq)]
pub struct Wave {
    pub label: Rect,
    pub prims: Vec<WavePrim>,
    /// Slot boundaries across the wave, for a faint time grid.
    pub ticks: Vec<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Slot {
    Low,
    High,
    Mid,
    Unknown,
    /// Data value number `n`.
    Data(usize),
    Clock { rising: bool },
}

fn slots(wave: &str) -> Vec<Slot> {
    let mut out: Vec<Slot> = Vec::new();
    let mut data = 0;
    for c in wave.chars() {
        let slot = match c {
            '0' | 'l' | 'L' | 'd' => Slot::Low,
            '1' | 'h' | 'H' | 'u' => Slot::High,
            'z' => Slot::Mid,
            'x' => Slot::Unknown,
            'p' | 'P' => Slot::Clock { rising: true },
            'n' | 'N' => Slot::Clock { rising: false },
            '=' | '2'..='9' => {
                data += 1;
                Slot::Data(data - 1)
            }
            '.' | '|' => match out.last() {
                Some(s) => *s,
                None => Slot::Unknown,
            },
            _ => continue,
        };
        out.push(slot);
    }
    out
}

/// Lay out `wave` in `r`, the label taking the left `label_w`.
pub fn layout(wave: &str, data: &[String], r: Rect, label_w: f64) -> Wave {
    let label = Rect::new(r.origin.x, r.origin.y, label_w, r.size.h);
    let slots = slots(wave);
    let n = slots.len().max(1);
    let x0 = r.origin.x + label_w;
    let cw = (r.size.w - label_w).max(1.0) / n as f64;
    let pad = (r.size.h * 0.18).min(10.0);
    let (hi, lo) = (r.origin.y + pad, r.origin.y + r.size.h - pad);
    let mid = (hi + lo) / 2.0;
    let skew = (cw * 0.2).min(5.0);
    let at = |i: usize| x0 + cw * i as f64;
    let ticks = (0..=n).map(at).collect();

    let mut prims = Vec::new();
    let mut line: Vec<Point> = Vec::new();
    // Where the last piece left the pen, for the next transition.
    let mut pen: Option<f64> = None;
    let flush = |line: &mut Vec<Point>, prims: &mut Vec<WavePrim>| {
        if line.len() > 1 {
            prims.push(WavePrim::Line(std::mem::take(line)));
        }
        line.clear();
    };
    let mut i = 0;
    while i < slots.len() {
        let s = slots[i];
        // A run: the same slot repeated (clocks go period by period).
        let mut j = i + 1;
        if !matches!(s, Slot::Clock { .. }) {
            while j < slots.len() && slots[j] == s {
                j += 1;
            }
        }
        let (a, b) = (at(i), at(j));
        let level = |y: f64, line: &mut Vec<Point>, pen: &mut Option<f64>| {
            match *pen {
                Some(p) if (p - y).abs() > f64::EPSILON => {
                    if line.is_empty() {
                        line.push(Point::new(a, p));
                    }
                    line.push(Point::new(a + skew, y));
                }
                Some(_) if !line.is_empty() => {}
                _ => line.push(Point::new(a, y)),
            }
            line.push(Point::new(b, y));
            *pen = Some(y);
        };
        match s {
            Slot::Low => level(lo, &mut line, &mut pen),
            Slot::High => level(hi, &mut line, &mut pen),
            Slot::Mid => level(mid, &mut line, &mut pen),
            Slot::Clock { rising } => {
                let (first, second) = if rising { (hi, lo) } else { (lo, hi) };
                let half = (a + b) / 2.0;
                let start = pen.unwrap_or(second);
                if line.is_empty() {
                    line.push(Point::new(a, start));
                }
                line.extend([Point::new(a + skew.min(cw * 0.1), first), Point::new(half, first), Point::new(half + skew.min(cw * 0.1), second), Point::new(b, second)]);
                pen = Some(second);
            }
            Slot::Unknown | Slot::Data(_) => {
                flush(&mut line, &mut prims);
                let points = vec![
                    Point::new(a, mid),
                    Point::new(a + skew, hi),
                    Point::new(b - skew, hi),
                    Point::new(b, mid),
                    Point::new(b - skew, lo),
                    Point::new(a + skew, lo),
                ];
                let label = match s {
                    Slot::Data(k) => data.get(k).cloned(),
                    _ => None,
                };
                prims.push(WavePrim::Bus { points, label, unknown: s == Slot::Unknown, center: Point::new((a + b) / 2.0, mid) });
                pen = Some(mid);
            }
        }
        i = j;
    }
    flush(&mut line, &mut prims);
    Wave { label, prims, ticks }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wave(s: &str, data: &[&str]) -> Wave {
        layout(s, &data.iter().map(|d| d.to_string()).collect::<Vec<_>>(), Rect::new(0.0, 0.0, 140.0, 40.0), 40.0)
    }

    #[test]
    fn levels_join_into_one_line() {
        let w = wave("01.0", &[]);
        assert_eq!(w.prims.len(), 1);
        let WavePrim::Line(pts) = &w.prims[0] else { panic!() };
        // Low, a slanted rise, high for two slots, a slanted fall, low.
        assert_eq!(pts.first().unwrap().x, 40.0);
        assert_eq!(pts.last().unwrap().x, 140.0);
        assert!(pts.iter().any(|p| p.y < 20.0) && pts.iter().any(|p| p.y > 20.0));
        assert_eq!(w.ticks.len(), 5);
    }

    #[test]
    fn data_spans_take_their_labels() {
        let w = wave("x=.=x", &["A", "B"]);
        let labels: Vec<Option<String>> = w.prims.iter().filter_map(|p| match p {
            WavePrim::Bus { label, .. } => Some(label.clone()),
            _ => None,
        }).collect();
        assert_eq!(labels, [None, Some("A".into()), Some("B".into()), None]);
    }

    #[test]
    fn clocks_tick_every_slot() {
        let w = wave("p...", &[]);
        let WavePrim::Line(pts) = &w.prims[0] else { panic!() };
        // Four periods: each rises and falls.
        let rises = pts.windows(2).filter(|p| p[1].y < p[0].y).count();
        assert_eq!(rises, 4);
    }
}
