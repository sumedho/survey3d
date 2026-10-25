//! GPU resources and the `egui_wgpu` paint callback.
//!
//! During `prepare` each viewport renders its scene into an offscreen,
//! multisampled colour target with its own reverse-Z depth buffer. During
//! `paint` the resolved image is drawn into egui's render pass. This keeps 3D
//! depth testing and MSAA independent of egui's own (depth-less) pass.

use std::collections::HashMap;
use std::num::NonZeroU64;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use egui_wgpu::wgpu::{self, util::DeviceExt as _};
use glam::{DVec3, Mat4};
use survey_core::Aabb3;

use crate::camera::{Camera, Projection};
use crate::clouds::CloudFrame;
use crate::scene::{LineInstance, PointInstance, Scene, TriVertex};

pub(crate) const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Capacity of the symbol table (must match `SymbolTable` in `scene.wgsl`).
const MAX_SYMBOLS: usize = 64;
const MAX_PRIM_VEC4S: usize = 256;
const MAX_PATTERNS: usize = 4;

/// Point symbols and line patterns from [`survey_core::style`], as one uniform block.
fn symbol_table() -> Vec<u8> {
    let (ranges, prims) = survey_core::style::gpu::table();
    assert!(
        ranges.len() <= MAX_SYMBOLS && prims.len() <= MAX_PRIM_VEC4S,
        "symbol table too large"
    );
    let mut r = vec![[0u32; 4]; MAX_SYMBOLS];
    r[..ranges.len()].copy_from_slice(&ranges);
    let mut p = vec![[0f32; 4]; MAX_PRIM_VEC4S];
    p[..prims.len()].copy_from_slice(&prims);
    let mut pat = [[0f32; 4]; MAX_PATTERNS];
    for style in survey_core::style::LineStyle::ALL {
        for (i, v) in style.pattern().iter().take(4).enumerate() {
            pat[style.id() as usize][i] = *v;
        }
    }
    let mut out = Vec::new();
    out.extend_from_slice(bytemuck::cast_slice(&r));
    out.extend_from_slice(bytemuck::cast_slice(&p));
    out.extend_from_slice(bytemuck::cast_slice(&pat));
    out
}

/// Per-draw uniforms. Stored at a dynamic offset per chunk.
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
struct Uniforms {
    mvp: [[f32; 4]; 4],
    viewport: [f32; 4],
    params: [f32; 4],
    color_a: [f32; 4],
    color_b: [f32; 4],
}

/// Plan grid drawn on the horizontal plane `z = elevation`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridSettings {
    pub elevation: f64,
    /// Straight RGBA in 0..1.
    pub minor_color: [f32; 4],
    pub major_color: [f32; 4],
    /// Smallest on-screen spacing (logical px) for minor lines.
    pub min_spacing_px: f64,
}

impl Default for GridSettings {
    fn default() -> Self {
        Self {
            elevation: 0.0,
            minor_color: [0.5, 0.55, 0.6, 0.18],
            major_color: [0.55, 0.6, 0.68, 0.38],
            min_spacing_px: 12.0,
        }
    }
}

impl GridSettings {
    /// Minor spacing (a power of ten) so that lines are at least `min_spacing_px` apart,
    /// plus the fade factor for minor lines as they approach that limit.
    pub fn spacing(&self, camera: &Camera) -> (f64, f32) {
        let wpp = camera.world_per_pixel().max(1e-9);
        let raw = wpp * self.min_spacing_px;
        let minor = 10f64.powf(raw.log10().ceil());
        let px = minor / wpp;
        let fade = ((px - self.min_spacing_px) / (self.min_spacing_px * 3.0)).clamp(0.0, 1.0);
        (minor, fade as f32 * 0.85 + 0.15)
    }

    /// Centre (snapped to a multiple of the major spacing) and half-extent of the grid quad.
    pub fn placement(&self, camera: &Camera) -> (DVec3, f64, f64) {
        let (minor, _) = self.spacing(camera);
        let major = minor * 10.0;
        let target = camera.target;
        let center = DVec3::new(
            (target.x / major).round() * major,
            (target.y / major).round() * major,
            self.elevation,
        );
        let view = camera.view_height() * camera.aspect().max(1.0);
        let reach = match camera.projection {
            Projection::Orthographic => view * 0.75,
            Projection::Perspective => view * 6.0,
        };
        (center, reach + major, minor)
    }

    /// World bounds of the grid quad (for fitting clip planes).
    pub fn bounds(&self, camera: &Camera) -> Aabb3 {
        let (c, half, _) = self.placement(camera);
        Aabb3::new(c - DVec3::new(half, half, 0.0), c + DVec3::new(half, half, 0.0))
    }
}

/// Everything needed to draw one viewport for one frame.
#[derive(Clone)]
pub struct ViewportFrame {
    pub camera: Camera,
    /// Scenes drawn together (e.g. drawing objects and surfaces); each is
    /// uploaded only when its revision changes.
    pub scenes: Vec<Arc<Scene>>,
    pub grid: Option<GridSettings>,
    /// Straight RGBA in 0..1.
    pub background: [f32; 4],
    /// Out-of-core point clouds (drawn on native targets only).
    pub clouds: Option<Arc<CloudFrame>>,
}

/// The egui paint callback for a viewport. Create with [`egui_wgpu::Callback::new_paint_callback`].
pub struct ViewportCallback {
    /// Distinguishes several viewports sharing the renderer.
    pub view_id: u64,
    pub size_px: [u32; 2],
    pub pixels_per_point: f32,
    pub frame: ViewportFrame,
    /// Called when level-of-detail points are still streaming in, so the
    /// host can schedule another frame (e.g. `ctx.request_repaint()`).
    pub request_frame: Option<Arc<dyn Fn() + Send + Sync>>,
}

/// Creates the renderer's shared GPU resources. Call once at startup.
pub fn install(render_state: &egui_wgpu::RenderState) {
    let resources = Resources::new(render_state);
    render_state.renderer.write().callback_resources.insert(resources);
}

struct Pipelines {
    point: wgpu::RenderPipeline,
    line: wgpu::RenderPipeline,
    tri: wgpu::RenderPipeline,
    grid: wgpu::RenderPipeline,
    blit: wgpu::RenderPipeline,
}

struct Resources {
    pipelines: Pipelines,
    uniform_layout: wgpu::BindGroupLayout,
    symbols: wgpu::Buffer,
    blit_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    color_format: wgpu::TextureFormat,
    samples: u32,
    uniform_stride: u64,
    views: HashMap<u64, ViewResources>,
    #[cfg(not(target_arch = "wasm32"))]
    clouds: crate::clouds::CloudResources,
}

struct GpuChunk {
    origin: DVec3,
    radius: f32,
    /// Resident point buffer and its length. For level-of-detail scenes this
    /// is a prefix of the chunk's shuffled points, uploaded on demand.
    points: Option<(wgpu::Buffer, u32)>,
    /// Points the chunk has in its scene.
    total: u32,
    /// Frame the chunk was last drawn (for eviction).
    last_used: u64,
    lines: Option<(wgpu::Buffer, u32)>,
    /// Vertex buffer and vertex count.
    triangles: Option<(wgpu::Buffer, u32)>,
    /// From a scene drawn on top of the others.
    on_top: bool,
}

/// The uploaded chunks of one scene.
struct Upload {
    revision: u64,
    lod: bool,
    /// Level-of-detail scenes are kept to stream points from.
    scene: Option<Arc<Scene>>,
    chunks: Vec<GpuChunk>,
}

/// A level-of-detail scene streaming in to replace an upload that is still
/// being drawn, so a new working set never shows half-loaded.
struct Staged {
    upload: Upload,
    /// Points each chunk will draw once promoted (planned every frame).
    counts: Vec<u32>,
    /// Frame staging began.
    since: u64,
}

/// Points drawn per square logical pixel of a level-of-detail chunk.
const LOD_DENSITY: f64 = 0.35;
/// Most level-of-detail points drawn in one frame.
const LOD_BUDGET: f64 = 4_000_000.0;
/// Most level-of-detail points uploaded in one frame; the rest follow over
/// the next frames, so a huge cloud never stalls a frame. Native GPUs take a
/// full working set in a frame or two (a replacement streams in behind the
/// scene it replaces, so this sets how soon new detail shows).
#[cfg(target_arch = "wasm32")]
const STREAM_PER_FRAME: u64 = 1_500_000;
#[cfg(not(target_arch = "wasm32"))]
const STREAM_PER_FRAME: u64 = 4_000_000;
/// Smallest streamed upload, so a chunk is not re-uploaded for every small zoom step.
const STREAM_MIN: u32 = 16_384;
/// Most frames a staged scene waits for its points before replacing the
/// drawn one anyway (a camera that never settles must not hold it off).
const STAGE_FRAMES: u64 = 30;
/// Level-of-detail points kept resident on the GPU per viewport before
/// chunks that are off screen are evicted (least recently drawn first).
#[cfg(target_arch = "wasm32")]
const RESIDENT_BUDGET: u64 = 8_000_000;
#[cfg(not(target_arch = "wasm32"))]
const RESIDENT_BUDGET: u64 = 24_000_000;

struct ViewResources {
    size: [u32; 2],
    msaa_view: Option<wgpu::TextureView>,
    resolve_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    blit_bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    uniform_slots: usize,
    uniform_bind_group: wgpu::BindGroup,
    uploads: Vec<Upload>,
    /// The replacement streaming in for each upload, if any (same indices).
    staged: Vec<Option<Staged>>,
    /// Points to draw per chunk this frame (all, or a level-of-detail prefix).
    point_counts: Vec<u32>,
    frame: u64,
    #[cfg(not(target_arch = "wasm32"))]
    clouds: crate::clouds::CloudView,
}

impl Resources {
    fn new(rs: &egui_wgpu::RenderState) -> Self {
        let device = &rs.device;
        // Draw the scene in the same (non-sRGB) space egui uses so colours match the UI.
        let color_format = rs.target_format.remove_srgb_suffix();
        let supports = |f: wgpu::TextureFormat, n| {
            rs.adapter
                .get_texture_format_features(f)
                .flags
                .sample_count_supported(n)
        };
        let samples = if supports(color_format, 4) && supports(DEPTH_FORMAT, 4) {
            4
        } else {
            1
        };
        let uniform_size = std::mem::size_of::<Uniforms>() as u64;
        let align = u64::from(device.limits().min_uniform_buffer_offset_alignment);
        let uniform_stride = uniform_size.div_ceil(align) * align;

        let table = symbol_table();
        let symbols = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("cad_render_symbols"),
            contents: &table,
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cad_render_uniforms"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: NonZeroU64::new(uniform_size),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(table.len() as u64),
                    },
                    count: None,
                },
            ],
        });
        let blit_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cad_render_blit"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("cad_render_blit"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let pipelines = Pipelines::new(
            device,
            &uniform_layout,
            &blit_layout,
            color_format,
            rs.target_format,
            samples,
        );
        Self {
            pipelines,
            uniform_layout,
            symbols,
            blit_layout,
            sampler,
            color_format,
            samples,
            uniform_stride,
            views: HashMap::new(),
            #[cfg(not(target_arch = "wasm32"))]
            clouds: crate::clouds::CloudResources::new(device, color_format, samples),
        }
    }

    /// Creates or resizes the offscreen targets for a viewport.
    fn ensure_view(&mut self, device: &wgpu::Device, id: u64, size: [u32; 2]) {
        let size = [size[0].max(1), size[1].max(1)];
        let stale = self.views.get(&id).is_none_or(|v| v.size != size);
        if stale {
            let previous = self.views.remove(&id);
            let mut view = ViewResources::new(self, device, size);
            if let Some(prev) = previous {
                // Keep uploaded geometry across resizes.
                view.uploads = prev.uploads;
                view.staged = prev.staged;
                view.point_counts = prev.point_counts;
                view.uniform_buffer = prev.uniform_buffer;
                view.uniform_slots = prev.uniform_slots;
                view.uniform_bind_group = prev.uniform_bind_group;
                #[cfg(not(target_arch = "wasm32"))]
                {
                    view.clouds = prev.clouds;
                }
            }
            self.views.insert(id, view);
        }
    }
}

impl ViewResources {
    fn new(res: &Resources, device: &wgpu::Device, size: [u32; 2]) -> Self {
        let extent = wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        };
        let make = |label, format, samples, usage| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: extent,
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let resolve_view = make(
            "cad_render_color",
            res.color_format,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let msaa_view = (res.samples > 1).then(|| {
            make(
                "cad_render_color_msaa",
                res.color_format,
                res.samples,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            )
        });
        let depth_view = make(
            "cad_render_depth",
            DEPTH_FORMAT,
            res.samples,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let blit_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cad_render_blit"),
            layout: &res.blit_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&resolve_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&res.sampler),
                },
            ],
        });
        let (uniform_buffer, uniform_bind_group) =
            create_uniforms(&res.uniform_layout, &res.symbols, res.uniform_stride, device, 16);
        Self {
            size,
            msaa_view,
            resolve_view,
            depth_view,
            blit_bind_group,
            uniform_buffer,
            uniform_slots: 16,
            uniform_bind_group,
            uploads: Vec::new(),
            staged: Vec::new(),
            point_counts: Vec::new(),
            frame: 0,
            #[cfg(not(target_arch = "wasm32"))]
            clouds: Default::default(),
        }
    }

    /// How many points of each chunk to draw: every point of ordinary scenes,
    /// and for level-of-detail scenes a number that suits the chunk's size on
    /// screen (none when it is off screen), within an overall budget. Staged
    /// scenes are planned the same way, for the points to stream ahead.
    fn plan_points(&mut self, camera: &Camera) {
        self.point_counts = plan(&self.uploads, camera);
        for s in self.staged.iter_mut().flatten() {
            s.counts = plan(std::slice::from_ref(&s.upload), camera);
        }
    }

    fn upload_scenes(&mut self, device: &wgpu::Device, scenes: &[Arc<Scene>]) {
        self.uploads.truncate(scenes.len());
        self.staged.truncate(scenes.len());
        for (i, scene) in scenes.iter().enumerate() {
            if self.uploads.get(i).is_some_and(|u| u.revision == scene.revision) {
                // Back to what is drawn: drop any replacement in flight.
                if let Some(s) = self.staged.get_mut(i) {
                    *s = None;
                }
                continue;
            }
            let staged = self.staged.get(i).and_then(Option::as_ref);
            if staged.is_some_and(|s| s.upload.revision == scene.revision) {
                continue;
            }
            let upload = new_upload(device, scene);
            // A level-of-detail scene replacing another streams in behind it;
            // anything else replaces at once (its geometry is uploaded whole).
            if scene.lod && self.uploads.get(i).is_some_and(|u| u.lod) {
                self.staged[i] = Some(Staged {
                    upload,
                    counts: Vec::new(),
                    since: self.frame,
                });
            } else if i < self.uploads.len() {
                self.uploads[i] = upload;
                self.staged[i] = None;
            } else {
                self.uploads.push(upload);
                self.staged.push(None);
            }
        }
    }

    /// Uploads the level-of-detail points this frame needs (largest needs
    /// first, within a per-frame budget) for drawn and staged scenes, and
    /// evicts off-screen chunks when the resident total exceeds the budget.
    /// Returns true when some wanted points are still to be uploaded.
    fn stream_points(&mut self, device: &wgpu::Device) -> bool {
        self.frame += 1;
        let frame = self.frame;
        // (want, upload index, staged?, chunk index)
        let mut requests = Vec::new();
        let mut want_from = |u: &mut Upload, counts: &[u32], ui: usize, staged: bool| {
            for (ci, c) in u.chunks.iter_mut().enumerate() {
                let want = counts.get(ci).copied().unwrap_or(0);
                if u.lod && want > 0 {
                    c.last_used = frame;
                    let resident = c.points.as_ref().map_or(0, |(_, n)| *n);
                    if want > resident {
                        requests.push((want, ui, staged, ci));
                    }
                }
            }
        };
        let mut flat = 0;
        for (ui, u) in self.uploads.iter_mut().enumerate() {
            let n = u.chunks.len();
            want_from(u, self.point_counts.get(flat..flat + n).unwrap_or(&[]), ui, false);
            flat += n;
        }
        for (ui, s) in self.staged.iter_mut().enumerate() {
            if let Some(s) = s {
                want_from(&mut s.upload, &s.counts, ui, true);
            }
        }
        requests.sort_by_key(|r| std::cmp::Reverse(r.0));
        let mut uploaded = 0u64;
        let mut pending = false;
        for (want, ui, staged, ci) in requests {
            if uploaded >= STREAM_PER_FRAME {
                pending = true;
                break;
            }
            let u = if staged {
                match &mut self.staged[ui] {
                    Some(s) => &mut s.upload,
                    None => continue,
                }
            } else {
                &mut self.uploads[ui]
            };
            let Some(scene) = &u.scene else { continue };
            let src = &scene.chunks[ci].points;
            // Over-fetch so small zooms are served from what is resident.
            let n = (want.saturating_mul(2)).max(STREAM_MIN).min(src.len() as u32);
            u.chunks[ci].points = instance_buffer(device, "cad_render_points_lod", &src[..n as usize]);
            uploaded += u64::from(n);
        }
        // Evict least recently drawn chunks that are not needed this frame.
        let mut all: Vec<&mut Upload> = self.uploads.iter_mut().collect();
        all.extend(self.staged.iter_mut().flatten().map(|s| &mut s.upload));
        all.retain(|u| u.lod);
        let mut resident: u64 = all
            .iter()
            .flat_map(|u| &u.chunks)
            .map(|c| u64::from(c.points.as_ref().map_or(0, |(_, n)| *n)))
            .sum();
        if resident > RESIDENT_BUDGET {
            let mut idle: Vec<(u64, usize, usize)> = Vec::new();
            for (ui, u) in all.iter().enumerate() {
                for (ci, c) in u.chunks.iter().enumerate() {
                    if c.points.is_some() && c.last_used < frame {
                        idle.push((c.last_used, ui, ci));
                    }
                }
            }
            idle.sort_unstable();
            for (_, ui, ci) in idle {
                if resident <= RESIDENT_BUDGET {
                    break;
                }
                if let Some((_, n)) = all[ui].chunks[ci].points.take() {
                    resident -= u64::from(n);
                }
            }
        }
        pending || self.staged.iter().any(Option::is_some)
    }

    /// Swaps in staged scenes whose wanted points are all resident (or that
    /// have waited long enough). Returns true when any was promoted, so the
    /// frame is planned again for what is now drawn.
    fn promote_staged(&mut self) -> bool {
        let mut promoted = false;
        for (i, slot) in self.staged.iter_mut().enumerate() {
            let ready = slot.as_ref().is_some_and(|s| {
                self.frame.saturating_sub(s.since) >= STAGE_FRAMES
                    || s.upload
                        .chunks
                        .iter()
                        .zip(&s.counts)
                        .all(|(c, want)| *want == 0 || c.points.as_ref().is_some_and(|(_, n)| n >= want))
            });
            if ready && let Some(s) = slot.take() {
                self.uploads[i] = s.upload;
                promoted = true;
            }
        }
        promoted
    }

    /// Draws only what is resident.
    fn clamp_counts(&mut self) {
        let mut flat = 0;
        for u in &self.uploads {
            for c in &u.chunks {
                if let Some(count) = self.point_counts.get_mut(flat) {
                    *count = (*count).min(c.points.as_ref().map_or(0, |(_, n)| *n));
                }
                flat += 1;
            }
        }
    }

    fn chunks(&self) -> impl Iterator<Item = &GpuChunk> {
        self.uploads.iter().flat_map(|u| &u.chunks)
    }
}

/// World units per logical pixel at a chunk (at its centre in perspective).
fn chunk_world_per_pixel(camera: &Camera, c: &GpuChunk) -> f64 {
    let wpp = match camera.projection {
        Projection::Orthographic => camera.world_per_pixel(),
        Projection::Perspective => {
            let d = (c.origin - camera.eye()).length().max(camera.near);
            camera.world_per_pixel() * d / camera.distance.max(1e-9)
        }
    };
    wpp.max(1e-12)
}

/// Points to draw per chunk of `uploads` (see [`ViewResources::plan_points`]).
fn plan(uploads: &[Upload], camera: &Camera) -> Vec<u32> {
    let mut counts = Vec::new();
    let mut lod_total = 0.0;
    let mut lod_slots = Vec::new();
    for u in uploads {
        for c in &u.chunks {
            let n = c.total;
            if !u.lod || n == 0 {
                counts.push(n);
                continue;
            }
            let r = f64::from(c.radius).max(1e-3);
            let r_px = r / chunk_world_per_pixel(camera, c);
            let inside_sphere = (c.origin - camera.eye()).length() < r;
            let visible = inside_sphere
                || camera.world_to_screen(c.origin).is_some_and(|(s, _)| {
                    s.x > -r_px
                        && s.y > -r_px
                        && s.x < camera.viewport.x + r_px
                        && s.y < camera.viewport.y + r_px
                });
            let want = if visible {
                (std::f64::consts::PI * r_px * r_px * LOD_DENSITY).min(n as f64)
            } else {
                0.0
            };
            lod_total += want;
            lod_slots.push(counts.len());
            counts.push(want as u32);
        }
    }
    if lod_total > LOD_BUDGET {
        let k = LOD_BUDGET / lod_total;
        for i in lod_slots {
            counts[i] = (f64::from(counts[i]) * k) as u32;
        }
    }
    counts
}

fn new_upload(device: &wgpu::Device, scene: &Arc<Scene>) -> Upload {
    Upload {
        revision: scene.revision,
        lod: scene.lod,
        scene: scene.lod.then(|| scene.clone()),
        chunks: scene
            .chunks
            .iter()
            .map(|c| GpuChunk {
                origin: c.origin,
                radius: c.radius,
                // Level-of-detail points stream in as the view needs them.
                points: if scene.lod {
                    None
                } else {
                    instance_buffer(device, "cad_render_points", &c.points)
                },
                total: c.points.len() as u32,
                last_used: 0,
                lines: instance_buffer(device, "cad_render_lines", &c.lines),
                triangles: instance_buffer(device, "cad_render_triangles", &c.triangles),
                on_top: scene.on_top,
            })
            .collect(),
    }
}

fn instance_buffer<T: Pod>(device: &wgpu::Device, label: &str, data: &[T]) -> Option<(wgpu::Buffer, u32)> {
    (!data.is_empty()).then(|| {
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::cast_slice(data),
            usage: wgpu::BufferUsages::VERTEX,
        });
        (buffer, data.len() as u32)
    })
}

fn create_uniforms(
    layout: &wgpu::BindGroupLayout,
    symbols: &wgpu::Buffer,
    stride: u64,
    device: &wgpu::Device,
    slots: usize,
) -> (wgpu::Buffer, wgpu::BindGroup) {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cad_render_uniforms"),
        size: stride * slots as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("cad_render_uniforms"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: NonZeroU64::new(std::mem::size_of::<Uniforms>() as u64),
                }),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: symbols.as_entire_binding(),
            },
        ],
    });
    (buffer, bind_group)
}

impl Pipelines {
    fn new(
        device: &wgpu::Device,
        uniform_layout: &wgpu::BindGroupLayout,
        blit_layout: &wgpu::BindGroupLayout,
        color_format: wgpu::TextureFormat,
        target_format: wgpu::TextureFormat,
        samples: u32,
    ) -> Self {
        let scene_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cad_render_scene"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/scene.wgsl").into()),
        });
        let blit_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cad_render_blit"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/blit.wgsl").into()),
        });
        let scene_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cad_render_scene"),
            bind_group_layouts: &[Some(uniform_layout)],
            immediate_size: 0,
        });
        let blit_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cad_render_blit"),
            bind_group_layouts: &[Some(blit_layout)],
            immediate_size: 0,
        });

        let blend = Some(wgpu::BlendState::ALPHA_BLENDING);
        let scene_pipeline = |label: &str,
                              vs: &str,
                              fs: &str,
                              buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
                              depth_write: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&scene_layout),
                vertex: wgpu::VertexState {
                    module: &scene_shader,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers,
                },
                fragment: Some(wgpu::FragmentState {
                    module: &scene_shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: color_format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(depth_write),
                    // Reverse-Z: larger depth is closer.
                    depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: samples,
                    mask: !0,
                    alpha_to_coverage_enabled: false,
                },
                multiview_mask: None,
                cache: None,
            })
        };

        let point_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32, 2 => Unorm8x4, 3 => Uint32];
        let line_attrs = wgpu::vertex_attr_array![
            0 => Float32x3, 1 => Float32, 2 => Float32x3, 3 => Unorm8x4, 4 => Float32, 5 => Uint32
        ];
        let point = scene_pipeline(
            "cad_render_points",
            "vs_point",
            "fs_point",
            &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<PointInstance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &point_attrs,
            })],
            true,
        );
        let line = scene_pipeline(
            "cad_render_lines",
            "vs_line",
            "fs_line",
            &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<LineInstance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &line_attrs,
            })],
            true,
        );
        let tri_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Unorm8x4];
        let tri = scene_pipeline(
            "cad_render_triangles",
            "vs_tri",
            "fs_tri",
            &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<TriVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &tri_attrs,
            })],
            true,
        );
        let grid = scene_pipeline("cad_render_grid", "vs_grid", "fs_grid", &[], false);

        let blit = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cad_render_blit"),
            layout: Some(&blit_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &blit_shader,
                entry_point: Some("vs_blit"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &blit_shader,
                entry_point: Some("fs_blit"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Self {
            point,
            line,
            tri,
            grid,
            blit,
        }
    }
}

fn uniforms(mvp: Mat4, viewport: [f32; 4]) -> Uniforms {
    Uniforms {
        mvp: mvp.to_cols_array_2d(),
        viewport,
        params: [0.0; 4],
        color_a: [0.0; 4],
        color_b: [0.0; 4],
    }
}

impl egui_wgpu::CallbackTrait for ViewportCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(res) = callback_resources.get_mut::<Resources>() else {
            log::error!("cad_render::install was not called");
            return Vec::new();
        };
        let stride = res.uniform_stride;
        let frame = &self.frame;
        let camera = &frame.camera;

        // Build uniforms in f64 first: slot 0 = grid, slots 1.. = chunks.
        let viewport = [
            self.size_px[0] as f32,
            self.size_px[1] as f32,
            self.pixels_per_point,
            0.0,
        ];
        let grid = frame.grid.map(|g| {
            let (center, half, minor) = g.placement(camera);
            let (_, minor_fade) = g.spacing(camera);
            let mut u = uniforms(camera.chunk_mvp(center), viewport);
            u.params = [minor as f32, (minor * 10.0) as f32, half as f32, 0.0];
            u.color_a = g.minor_color;
            u.color_a[3] *= minor_fade;
            u.color_b = g.major_color;
            u
        });

        res.ensure_view(device, self.view_id, self.size_px);
        let Resources {
            pipelines,
            views,
            uniform_layout,
            symbols,
            #[cfg(not(target_arch = "wasm32"))]
            clouds,
            ..
        } = res;
        let Some(view) = views.get_mut(&self.view_id) else {
            return Vec::new();
        };
        view.upload_scenes(device, &frame.scenes);
        view.plan_points(camera);
        let streaming = view.stream_points(device);
        if view.promote_staged() {
            view.plan_points(camera);
        }
        view.clamp_counts();
        // Out-of-core clouds render into their own targets first; the main
        // pass composites them (with eye-dome lighting) before anything else.
        #[cfg(not(target_arch = "wasm32"))]
        let streaming = {
            let empty = CloudFrame::default();
            let cloud_frame = frame.clouds.as_deref().unwrap_or(&empty);
            let size = [self.size_px[0].max(1), self.size_px[1].max(1)];
            view.clouds.prepare(
                clouds,
                device,
                queue,
                encoder,
                camera,
                cloud_frame,
                size,
                self.pixels_per_point,
            ) || streaming
        };
        if streaming && let Some(request) = &self.request_frame {
            request();
        }
        // Slot 0 = grid, then one per drawn chunk.
        let needed_slots = 1 + view.chunks().count();
        if view.uniform_slots < needed_slots {
            let slots = needed_slots.next_power_of_two();
            let (buffer, bind_group) = create_uniforms(uniform_layout, symbols, stride, device, slots);
            view.uniform_buffer = buffer;
            view.uniform_bind_group = bind_group;
            view.uniform_slots = slots;
        }

        let mut data = vec![0u8; stride as usize * needed_slots];
        let mut put = |slot: usize, u: &Uniforms| {
            let start = slot * stride as usize;
            data[start..start + std::mem::size_of::<Uniforms>()].copy_from_slice(bytemuck::bytes_of(u));
        };
        if let Some(g) = &grid {
            put(0, g);
        }
        // params.w flags plan view: surfaces then sit behind everything drawn on them.
        // params.z flags highlight scenes, drawn in front of the same geometry elsewhere.
        let plan = if camera.is_plan() { 1.0 } else { 0.0 };
        for (i, chunk) in view.chunks().enumerate() {
            let mut u = uniforms(camera.chunk_mvp(chunk.origin), viewport);
            u.params[2] = if chunk.on_top { 1.0 } else { 0.0 };
            u.params[3] = plan;
            put(i + 1, &u);
        }
        queue.write_buffer(&view.uniform_buffer, 0, &data);

        let bg = frame.background;
        let (color_view, resolve_target) = match &view.msaa_view {
            Some(msaa) => (msaa, Some(&view.resolve_view)),
            None => (&view.resolve_view, None),
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("cad_render_scene"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color_view,
                depth_slice: None,
                resolve_target,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: f64::from(bg[0]),
                        g: f64::from(bg[1]),
                        b: f64::from(bg[2]),
                        a: f64::from(bg[3]),
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &view.depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

        #[cfg(not(target_arch = "wasm32"))]
        view.clouds.composite(clouds, &mut pass);
        if grid.is_some() {
            pass.set_pipeline(&pipelines.grid);
            pass.set_bind_group(0, &view.uniform_bind_group, &[0]);
            pass.draw(0..6, 0..1);
        }
        pass.set_pipeline(&pipelines.tri);
        for (i, chunk) in view.chunks().enumerate() {
            if let Some((buf, n)) = &chunk.triangles {
                pass.set_bind_group(0, &view.uniform_bind_group, &[((i + 1) as u64 * stride) as u32]);
                pass.set_vertex_buffer(0, buf.slice(..));
                pass.draw(0..*n, 0..1);
            }
        }
        pass.set_pipeline(&pipelines.line);
        for (i, chunk) in view.chunks().enumerate() {
            if let Some((buf, n)) = &chunk.lines {
                pass.set_bind_group(0, &view.uniform_bind_group, &[((i + 1) as u64 * stride) as u32]);
                pass.set_vertex_buffer(0, buf.slice(..));
                pass.draw(0..6, 0..*n);
            }
        }
        pass.set_pipeline(&pipelines.point);
        for (i, chunk) in view.chunks().enumerate() {
            if let Some((buf, _)) = &chunk.points {
                let n = &view.point_counts.get(i).copied().unwrap_or(0);
                if *n == 0 {
                    continue;
                }
                pass.set_bind_group(0, &view.uniform_bind_group, &[((i + 1) as u64 * stride) as u32]);
                pass.set_vertex_buffer(0, buf.slice(..));
                pass.draw(0..6, 0..*n);
            }
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &egui_wgpu::CallbackResources,
    ) {
        let Some(res) = callback_resources.get::<Resources>() else {
            return;
        };
        let Some(view) = res.views.get(&self.view_id) else {
            return;
        };
        render_pass.set_pipeline(&res.pipelines.blit);
        render_pass.set_bind_group(0, &view.blit_bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    /// Validates the shaders and checks they translate to GLSL ES 3.0 (the WebGL2 path).
    #[test]
    fn shaders_validate_for_webgl2() {
        for (name, src) in [
            ("scene", include_str!("shaders/scene.wgsl")),
            ("blit", include_str!("shaders/blit.wgsl")),
        ] {
            let module = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{name}: {e}"));
            let info = naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::empty(),
            )
            .validate(&module)
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
            for ep in &module.entry_points {
                let options = naga::back::glsl::Options {
                    version: naga::back::glsl::Version::Embedded {
                        version: 300,
                        is_webgl: true,
                    },
                    ..Default::default()
                };
                let pipeline = naga::back::glsl::PipelineOptions {
                    shader_stage: ep.stage,
                    entry_point: ep.name.clone(),
                    multiview: None,
                };
                let mut out = String::new();
                let mut writer = naga::back::glsl::Writer::new(
                    &mut out,
                    &module,
                    &info,
                    &options,
                    &pipeline,
                    naga::proc::BoundsCheckPolicies::default(),
                )
                .unwrap_or_else(|e| panic!("{name}/{}: {e}", ep.name));
                writer
                    .write()
                    .unwrap_or_else(|e| panic!("{name}/{}: {e}", ep.name));
            }
        }
    }

    /// The point-cloud shaders are native only (a storage buffer in the vertex
    /// stage): they only need to validate.
    #[test]
    fn cloud_shaders_validate() {
        for (name, src) in [
            ("clouds", include_str!("shaders/clouds.wgsl")),
            ("cloud_edl", include_str!("shaders/cloud_edl.wgsl")),
        ] {
            let module = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{name}: {e}"));
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        }
    }
}
