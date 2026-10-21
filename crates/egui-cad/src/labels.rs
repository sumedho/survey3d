//! Collision-avoiding placement of screen labels (point names, codes, heights).
//!
//! Labels are offered in priority order; each is placed only if its rectangle
//! does not overlap one already placed. Occupancy is tracked in a coarse grid,
//! so placing thousands of labels stays cheap.

use egui::{Pos2, Rect, Vec2};

pub struct LabelPlacer {
    bounds: Rect,
    cell: f32,
    cols: usize,
    rows: usize,
    placed: Vec<Vec<Rect>>,
    count: usize,
    limit: usize,
}

impl LabelPlacer {
    /// `limit` caps how many labels are placed (keeps dense views responsive).
    pub fn new(bounds: Rect, limit: usize) -> Self {
        let cell = 48.0;
        let cols = ((bounds.width() / cell).ceil() as usize).max(1);
        let rows = ((bounds.height() / cell).ceil() as usize).max(1);
        Self {
            bounds,
            cell,
            cols,
            rows,
            placed: vec![Vec::new(); cols * rows],
            count: 0,
            limit,
        }
    }

    pub fn is_full(&self) -> bool {
        self.count >= self.limit
    }

    fn cells(&self, r: Rect) -> impl Iterator<Item = usize> + '_ {
        let to_cell =
            |v: f32, origin: f32, n: usize| (((v - origin) / self.cell).floor().max(0.0) as usize).min(n - 1);
        let (c0, c1) = (
            to_cell(r.min.x, self.bounds.min.x, self.cols),
            to_cell(r.max.x, self.bounds.min.x, self.cols),
        );
        let (r0, r1) = (
            to_cell(r.min.y, self.bounds.min.y, self.rows),
            to_cell(r.max.y, self.bounds.min.y, self.rows),
        );
        (r0..=r1).flat_map(move |row| (c0..=c1).map(move |col| row * self.cols + col))
    }

    /// Tries to place a label of `size` with its top-left at `anchor + offset`.
    /// Returns the rectangle if it fits inside the bounds without overlapping.
    pub fn place(&mut self, anchor: Pos2, offset: Vec2, size: Vec2) -> Option<Rect> {
        if self.is_full() {
            return None;
        }
        let rect = Rect::from_min_size(anchor + offset, size);
        if !self.bounds.contains_rect(rect) {
            return None;
        }
        let padded = rect.expand(2.0);
        if self
            .cells(padded)
            .any(|i| self.placed[i].iter().any(|r| r.intersects(padded)))
        {
            return None;
        }
        let cells: Vec<usize> = self.cells(padded).collect();
        for i in cells {
            self.placed[i].push(rect);
        }
        self.count += 1;
        Some(rect)
    }

    /// Marks an area (e.g. a point marker) as occupied without counting it as a label.
    pub fn reserve(&mut self, rect: Rect) {
        let cells: Vec<usize> = self.cells(rect).collect();
        for i in cells {
            self.placed[i].push(rect);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2, vec2};

    #[test]
    fn overlapping_labels_are_culled() {
        let mut p = LabelPlacer::new(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0)), 100);
        let size = vec2(60.0, 14.0);
        assert!(p.place(pos2(10.0, 10.0), Vec2::ZERO, size).is_some());
        assert!(
            p.place(pos2(30.0, 15.0), Vec2::ZERO, size).is_none(),
            "overlaps the first"
        );
        assert!(p.place(pos2(10.0, 40.0), Vec2::ZERO, size).is_some());
        assert!(
            p.place(pos2(390.0, 10.0), Vec2::ZERO, size).is_none(),
            "outside bounds"
        );
        p.reserve(Rect::from_min_size(pos2(200.0, 200.0), vec2(10.0, 10.0)));
        assert!(
            p.place(pos2(195.0, 195.0), Vec2::ZERO, size).is_none(),
            "reserved area"
        );
    }

    #[test]
    fn label_beside_its_reserved_marker_fits() {
        let mut p = LabelPlacer::new(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0)), 100);
        let anchor = pos2(100.0, 100.0);
        p.reserve(Rect::from_center_size(anchor, vec2(6.0, 6.0)));
        assert!(p.place(anchor, vec2(7.0, -7.0), vec2(40.0, 14.0)).is_some());
    }

    #[test]
    fn limit_is_respected() {
        let mut p = LabelPlacer::new(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 1000.0)), 2);
        let size = vec2(10.0, 10.0);
        assert!(p.place(pos2(0.0, 0.0), Vec2::ZERO, size).is_some());
        assert!(p.place(pos2(100.0, 0.0), Vec2::ZERO, size).is_some());
        assert!(p.place(pos2(200.0, 0.0), Vec2::ZERO, size).is_none());
        assert!(p.is_full());
    }
}
