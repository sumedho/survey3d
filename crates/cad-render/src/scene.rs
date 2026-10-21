//! CPU-side scene description.
//!
//! Geometry is bucketed into spatial chunks. Each chunk has an `f64` origin and
//! stores `f32` offsets from it, so vertex positions stay small (≤ a chunk size)
//! no matter how large the world coordinates are.

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use glam::DVec3;
use survey_core::Aabb3;

/// Default chunk edge length in metres. f32 offsets within 2 km keep ~0.1 mm precision.
pub const DEFAULT_CHUNK_SIZE: f64 = 2048.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum PointShape {
    Circle = 0,
    Square = 1,
    Cross = 2,
}

/// GPU instance for a screen-space point marker.
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
pub struct PointInstance {
    pub offset: [f32; 3],
    pub size_px: f32,
    /// RGBA8, straight alpha.
    pub color: [u8; 4],
    pub shape: u32,
}

/// GPU instance for a screen-space-width line segment.
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
pub struct LineInstance {
    pub a: [f32; 3],
    pub width_px: f32,
    pub b: [f32; 3],
    pub color: [u8; 4],
}

#[derive(Clone, Debug)]
pub struct Chunk {
    pub origin: DVec3,
    pub points: Vec<PointInstance>,
    pub lines: Vec<LineInstance>,
}

/// An immutable, renderable scene. Rebuild (cheaply) when the document changes.
#[derive(Clone, Debug, Default)]
pub struct Scene {
    pub chunks: Vec<Chunk>,
    pub bounds: Aabb3,
    /// Identifies this scene's content; the renderer re-uploads when it changes.
    pub revision: u64,
}

impl Scene {
    pub fn point_count(&self) -> usize {
        self.chunks.iter().map(|c| c.points.len()).sum()
    }

    pub fn line_count(&self) -> usize {
        self.chunks.iter().map(|c| c.lines.len()).sum()
    }
}

pub struct SceneBuilder {
    chunk_size: f64,
    index: HashMap<[i64; 3], usize>,
    chunks: Vec<Chunk>,
    bounds: Aabb3,
}

impl Default for SceneBuilder {
    fn default() -> Self {
        Self::new(DEFAULT_CHUNK_SIZE)
    }
}

impl SceneBuilder {
    pub fn new(chunk_size: f64) -> Self {
        Self {
            chunk_size,
            index: HashMap::new(),
            chunks: Vec::new(),
            bounds: Aabb3::EMPTY,
        }
    }

    fn chunk_for(&mut self, p: DVec3) -> &mut Chunk {
        let key = (p / self.chunk_size).floor().as_i64vec3().to_array();
        let size = self.chunk_size;
        let i = *self.index.entry(key).or_insert_with(|| {
            let origin =
                DVec3::new(key[0] as f64, key[1] as f64, key[2] as f64) * size + DVec3::splat(size * 0.5);
            self.chunks.push(Chunk {
                origin,
                points: Vec::new(),
                lines: Vec::new(),
            });
            self.chunks.len() - 1
        });
        &mut self.chunks[i]
    }

    pub fn add_point(&mut self, p: DVec3, color: [u8; 4], size_px: f32, shape: PointShape) {
        self.bounds.extend(p);
        let chunk = self.chunk_for(p);
        let offset = (p - chunk.origin).as_vec3().to_array();
        chunk.points.push(PointInstance {
            offset,
            size_px,
            color,
            shape: shape as u32,
        });
    }

    pub fn add_line(&mut self, a: DVec3, b: DVec3, color: [u8; 4], width_px: f32) {
        self.bounds.extend(a);
        self.bounds.extend(b);
        let chunk = self.chunk_for((a + b) * 0.5);
        let origin = chunk.origin;
        chunk.lines.push(LineInstance {
            a: (a - origin).as_vec3().to_array(),
            width_px,
            b: (b - origin).as_vec3().to_array(),
            color,
        });
    }

    pub fn build(self, revision: u64) -> Scene {
        Scene {
            chunks: self.chunks,
            bounds: self.bounds,
            revision,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_are_small_at_large_coordinates() {
        let mut b = SceneBuilder::default();
        let p = DVec3::new(512_345.678_9, 6_912_345.678_9, 101.234_5);
        b.add_point(p, [255; 4], 6.0, PointShape::Circle);
        b.add_line(p, p + DVec3::new(1.0, 0.0, 0.0), [255; 4], 2.0);
        let scene = b.build(1);
        assert_eq!(scene.point_count(), 1);
        let chunk = &scene.chunks[0];
        let off = chunk.points[0].offset;
        assert!(off.iter().all(|v| v.abs() <= DEFAULT_CHUNK_SIZE as f32));
        let back = chunk.origin + DVec3::new(f64::from(off[0]), f64::from(off[1]), f64::from(off[2]));
        assert!((back - p).length() < 1e-4, "sub-0.1 mm reconstruction");
    }
}
