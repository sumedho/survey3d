//! Geoid models: grids of the geoid–ellipsoid separation N, so that
//! ellipsoidal height h = orthometric height H + N.
//!
//! A model is one or more regular latitude/longitude grids (NTv2 and PROJ
//! GeoTIFF files may hold several, a finer one inside a coarser one).
//! Separations are interpolated bilinearly between the four nodes around a
//! point; nodes without data make the point unknown.

/// One regular grid of separations.
#[derive(Clone, Debug, PartialEq)]
pub struct GeoidGrid {
    /// Latitude and longitude of the south-west node (degrees).
    pub south: f64,
    pub west: f64,
    /// Node spacing (degrees).
    pub dlat: f64,
    pub dlon: f64,
    pub rows: usize,
    pub cols: usize,
    /// Separations (metres), row by row from the south, west to east in each
    /// row; NaN where the grid has no data.
    pub values: Vec<f32>,
}

impl GeoidGrid {
    /// A grid, checked for a consistent size and spacing.
    pub fn new(
        south: f64,
        west: f64,
        dlat: f64,
        dlon: f64,
        rows: usize,
        cols: usize,
        values: Vec<f32>,
    ) -> Result<Self, String> {
        if rows < 2 || cols < 2 {
            return Err(format!("a grid needs at least 2×2 nodes (has {rows}×{cols})"));
        }
        if values.len() != rows * cols {
            return Err(format!("{} values for a {rows}×{cols} grid", values.len()));
        }
        if !(dlat > 0.0 && dlon > 0.0 && south.is_finite() && west.is_finite()) {
            return Err("the grid spacing must be positive".into());
        }
        Ok(Self {
            south,
            west,
            dlat,
            dlon,
            rows,
            cols,
            values,
        })
    }

    pub fn north(&self) -> f64 {
        self.south + self.dlat * (self.rows - 1) as f64
    }

    pub fn east(&self) -> f64 {
        self.west + self.dlon * (self.cols - 1) as f64
    }

    /// Whether the grid goes all the way round (its columns wrap).
    fn global(&self) -> bool {
        self.dlon * self.cols as f64 >= 360.0 - 1e-9
    }

    fn at(&self, row: usize, col: usize) -> f64 {
        f64::from(self.values[row * self.cols + col])
    }

    /// The separation at a latitude and longitude (degrees), if the grid
    /// covers it with data.
    pub fn separation(&self, lat: f64, lon: f64) -> Option<f64> {
        let y = (lat - self.south) / self.dlat;
        if !(-1e-9..=(self.rows - 1) as f64 + 1e-9).contains(&y) {
            return None;
        }
        let (c0, c1, fx) = if self.global() {
            let x = (lon - self.west).rem_euclid(360.0) / self.dlon;
            let c0 = (x.floor() as usize).min(self.cols - 1);
            (c0, (c0 + 1) % self.cols, x - c0 as f64)
        } else {
            // Longitudes may be given in −180…180 or 0…360.
            let x = [lon, lon + 360.0, lon - 360.0]
                .into_iter()
                .map(|l| (l - self.west) / self.dlon)
                .find(|x| (-1e-9..=(self.cols - 1) as f64 + 1e-9).contains(x))?;
            let c0 = (x.floor().max(0.0) as usize).min(self.cols - 2);
            (c0, c0 + 1, x - c0 as f64)
        };
        let r0 = (y.floor().max(0.0) as usize).min(self.rows - 2);
        let fy = y - r0 as f64;
        let (a, b) = (self.at(r0, c0), self.at(r0, c1));
        let (c, d) = (self.at(r0 + 1, c0), self.at(r0 + 1, c1));
        let v = (a * (1.0 - fx) + b * fx) * (1.0 - fy) + (c * (1.0 - fx) + d * fx) * fy;
        v.is_finite().then_some(v)
    }
}

/// A geoid model: its name and grids, finest first.
#[derive(Clone, Debug, PartialEq)]
pub struct GeoidModel {
    pub name: String,
    grids: Vec<GeoidGrid>,
}

impl GeoidModel {
    pub fn new(name: impl Into<String>, mut grids: Vec<GeoidGrid>) -> Result<Self, String> {
        if grids.is_empty() {
            return Err("the file holds no geoid grid".into());
        }
        grids.sort_by(|a, b| (a.dlat * a.dlon).total_cmp(&(b.dlat * b.dlon)));
        Ok(Self {
            name: name.into(),
            grids,
        })
    }

    pub fn grids(&self) -> &[GeoidGrid] {
        &self.grids
    }

    /// The separation N (metres) at a latitude and longitude (degrees), from
    /// the finest grid with data there.
    pub fn separation(&self, lat: f64, lon: f64) -> Option<f64> {
        self.grids.iter().find_map(|g| g.separation(lat, lon))
    }

    /// The area covered, as (south, west, north, east) degrees.
    pub fn extent(&self) -> (f64, f64, f64, f64) {
        self.grids.iter().fold(
            (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY),
            |(s, w, n, e), g| (s.min(g.south), w.min(g.west), n.max(g.north()), e.max(g.east())),
        )
    }

    /// The node spacing of the finest grid, in arc minutes.
    pub fn resolution_minutes(&self) -> f64 {
        self.grids[0].dlat * 60.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// N = lat + 2·lon over a small grid: bilinear interpolation is exact.
    fn plane(south: f64, west: f64, step: f64, rows: usize, cols: usize) -> GeoidGrid {
        let values = (0..rows)
            .flat_map(|r| {
                (0..cols).map(move |c| (south + r as f64 * step + 2.0 * (west + c as f64 * step)) as f32)
            })
            .collect();
        GeoidGrid::new(south, west, step, step, rows, cols, values).expect("value")
    }

    #[test]
    fn interpolates_inside_and_refuses_outside() {
        let g = plane(-38.0, 144.0, 0.5, 5, 5);
        let n = g.separation(-37.3, 145.2).expect("value");
        assert!((n - (-37.3 + 2.0 * 145.2)).abs() < 1e-4, "{n}");
        // The north-east corner node itself.
        assert!((g.separation(-36.0, 146.0).expect("value") - (-36.0 + 292.0)).abs() < 1e-4);
        assert!(g.separation(-35.9, 145.0).is_none());
        assert!(g.separation(-37.0, 146.1).is_none());
        // Longitudes given as 0…360 or −180…180.
        let w = plane(-10.0, -60.0, 1.0, 3, 3);
        assert_eq!(w.separation(-9.0, -59.0), w.separation(-9.0, 301.0));
    }

    #[test]
    fn missing_nodes_and_the_finest_grid() {
        let mut coarse = plane(-40.0, 140.0, 1.0, 6, 6);
        coarse.values[0] = f32::NAN;
        assert!(coarse.separation(-39.9, 140.1).is_none());
        let mut fine = plane(-38.0, 142.0, 0.25, 5, 5);
        for v in &mut fine.values {
            *v += 1.0;
        }
        let model = GeoidModel::new("test", vec![coarse, fine]).expect("value");
        // Inside the fine grid its values (offset by 1) win; outside, the coarse grid's.
        assert!((model.separation(-37.5, 142.5).expect("value") - (-37.5 + 285.0 + 1.0)).abs() < 1e-4);
        assert!((model.separation(-36.0, 144.0).expect("value") - (-36.0 + 288.0)).abs() < 1e-4);
        assert_eq!(model.extent(), (-40.0, 140.0, -35.0, 145.0));
    }

    #[test]
    fn global_grids_wrap() {
        // 1° grid round the world: columns 0…359.
        let values = (0..3).flat_map(|_| (0..360).map(|c| c as f32)).collect();
        let g = GeoidGrid::new(-1.0, 0.0, 1.0, 1.0, 3, 360, values).expect("value");
        // Between 359° (value 359) and 0° (value 0, wrapped).
        assert!((g.separation(0.0, 359.5).expect("value") - 179.5).abs() < 1e-6);
        assert!((g.separation(0.0, -0.5).expect("value") - 179.5).abs() < 1e-6);
        assert!((g.separation(0.0, 10.25).expect("value") - 10.25).abs() < 1e-6);
    }
}
