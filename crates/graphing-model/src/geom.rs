#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Size {
    pub w: f64,
    pub h: f64,
}

impl Size {
    pub const fn new(w: f64, h: f64) -> Self {
        Self { w, h }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rect {
    pub origin: Point,
    pub size: Size,
}

impl Rect {
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { origin: Point::new(x, y), size: Size::new(w, h) }
    }

    pub fn center(&self) -> Point {
        Point::new(self.origin.x + self.size.w / 2.0, self.origin.y + self.size.h / 2.0)
    }

    /// Grown by `by` on every side.
    pub fn inflate(&self, by: f64) -> Rect {
        Rect::new(self.origin.x - by, self.origin.y - by, self.size.w + by * 2.0, self.size.h + by * 2.0)
    }

    /// The smallest rect holding both.
    pub fn union(&self, o: Rect) -> Rect {
        let (x0, y0) = (self.origin.x.min(o.origin.x), self.origin.y.min(o.origin.y));
        let x1 = (self.origin.x + self.size.w).max(o.origin.x + o.size.w);
        let y1 = (self.origin.y + self.size.h).max(o.origin.y + o.size.h);
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }

    /// The smallest rect holding every point; `None` for none.
    pub fn around(points: impl IntoIterator<Item = Point>) -> Option<Rect> {
        points.into_iter().map(|p| Rect::new(p.x, p.y, 0.0, 0.0)).reduce(|a, b| a.union(b))
    }

    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.origin.x
            && p.y >= self.origin.y
            && p.x <= self.origin.x + self.size.w
            && p.y <= self.origin.y + self.size.h
    }

    /// Where the ray from the center towards `toward` leaves this rect.
    pub fn boundary_toward(&self, toward: Point) -> Point {
        let c = self.center();
        let (dx, dy) = (toward.x - c.x, toward.y - c.y);
        if dx == 0.0 && dy == 0.0 {
            return c;
        }
        let (hw, hh) = (self.size.w / 2.0, self.size.h / 2.0);
        let tx = if dx != 0.0 { hw / dx.abs() } else { f64::INFINITY };
        let ty = if dy != 0.0 { hh / dy.abs() } else { f64::INFINITY };
        let t = tx.min(ty);
        Point::new(c.x + dx * t, c.y + dy * t)
    }
}
