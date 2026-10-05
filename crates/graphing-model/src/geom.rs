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
