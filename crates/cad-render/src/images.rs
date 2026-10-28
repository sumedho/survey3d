//! Georeferenced images drawn as textured tiles under the drawing. The host
//! decides which tiles a view needs (and at what resolution) and hands them
//! over with their pixels; each tile is uploaded once and kept while it is
//! used, so panning only uploads the tiles that come into view.

use std::collections::HashMap;
use std::num::NonZeroU64;
use std::sync::Arc;

use egui_wgpu::wgpu;
use glam::DVec3;

use crate::camera::Camera;
use crate::renderer::DEPTH_FORMAT;

/// A tile's pixels (straight RGBA, rows from the top).
#[derive(Debug)]
pub struct ImagePixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// One textured quad: (part of) a tile placed in the world.
#[derive(Clone, Debug)]
pub struct ImageQuad {
    /// Identifies the tile's pixels: uploaded once per key.
    pub key: u64,
    pub pixels: Arc<ImagePixels>,
    /// World corners: top-left, top-right, bottom-right, bottom-left.
    pub corners: [DVec3; 4],
    /// The part of the tile drawn: u0, v0, u1, v1 (0–1).
    pub uv: [f32; 4],
    pub opacity: f32,
}

/// The tiles one frame draws, in order (first underneath).
#[derive(Clone, Debug, Default)]
pub struct ImageFrame {
    pub quads: Vec<ImageQuad>,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 3],
    uv: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    mvp: [[f32; 4]; 4],
    params: [f32; 4],
}

struct GpuTile {
    bind_group: wgpu::BindGroup,
    last_used: u64,
    bytes: u64,
}

/// Uniforms and vertices of one viewport's quads this frame.
struct ViewImages {
    uniforms: wgpu::Buffer,
    uniform_group: wgpu::BindGroup,
    slots: usize,
    vertices: Option<wgpu::Buffer>,
    /// The tile key of each quad drawn.
    keys: Vec<u64>,
}

/// GPU memory kept for tiles before the least recently used are released.
const TEXTURE_BUDGET: u64 = 512 * 1024 * 1024;

pub(crate) struct ImageResources {
    pipeline: wgpu::RenderPipeline,
    uniform_layout: wgpu::BindGroupLayout,
    texture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    stride: u64,
    tiles: HashMap<u64, GpuTile>,
    resident: u64,
    frame: u64,
    views: HashMap<u64, ViewImages>,
}

impl ImageResources {
    pub(crate) fn new(device: &wgpu::Device, color_format: wgpu::TextureFormat, samples: u32) -> Self {
        let size = std::mem::size_of::<Uniforms>() as u64;
        let align = u64::from(device.limits().min_uniform_buffer_offset_alignment);
        let stride = size.div_ceil(align) * align;
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cad_render_image_uniforms"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: NonZeroU64::new(size),
                },
                count: None,
            }],
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cad_render_image_tile"),
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
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cad_render_image"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/image.wgsl").into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cad_render_image"),
            bind_group_layouts: &[Some(&uniform_layout), Some(&texture_layout)],
            immediate_size: 0,
        });
        let attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cad_render_image"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_image"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attrs,
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_image"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            // Under the drawing: tested against what is drawn, never hiding it.
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
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
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("cad_render_image"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            pipeline,
            uniform_layout,
            texture_layout,
            sampler,
            stride,
            tiles: HashMap::new(),
            resident: 0,
            frame: 0,
            views: HashMap::new(),
        }
    }

    fn uniform_buffer(&self, device: &wgpu::Device, slots: usize) -> (wgpu::Buffer, wgpu::BindGroup) {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("cad_render_image_uniforms"),
            size: self.stride * slots as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cad_render_image_uniforms"),
            layout: &self.uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: NonZeroU64::new(std::mem::size_of::<Uniforms>() as u64),
                }),
            }],
        });
        (buffer, group)
    }

    fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, key: u64, px: &ImagePixels) {
        if let Some(t) = self.tiles.get_mut(&key) {
            t.last_used = self.frame;
            return;
        }
        if px.width == 0 || px.height == 0 || px.rgba.len() < (px.width * px.height * 4) as usize {
            return;
        }
        let size = wgpu::Extent3d {
            width: px.width,
            height: px.height,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cad_render_image_tile"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &px.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(px.width * 4),
                rows_per_image: Some(px.height),
            },
            size,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cad_render_image_tile"),
            layout: &self.texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        let bytes = u64::from(px.width) * u64::from(px.height) * 4;
        self.resident += bytes;
        self.tiles.insert(
            key,
            GpuTile {
                bind_group,
                last_used: self.frame,
                bytes,
            },
        );
    }

    /// Releases the least recently used tiles over the budget (never ones
    /// used this frame).
    fn evict(&mut self) {
        if self.resident <= TEXTURE_BUDGET {
            return;
        }
        let mut old: Vec<(u64, u64)> = self
            .tiles
            .iter()
            .filter(|(_, t)| t.last_used < self.frame)
            .map(|(k, t)| (t.last_used, *k))
            .collect();
        old.sort_unstable();
        for (_, key) in old {
            if self.resident <= TEXTURE_BUDGET * 3 / 4 {
                break;
            }
            if let Some(t) = self.tiles.remove(&key) {
                self.resident -= t.bytes;
            }
        }
    }

    pub(crate) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view_id: u64,
        camera: &Camera,
        frame: Option<&ImageFrame>,
    ) {
        self.frame += 1;
        let quads = frame.map_or(&[][..], |f| &f.quads[..]);
        for q in quads {
            self.upload(device, queue, q.key, &q.pixels);
        }
        self.evict();
        let n = quads.len();
        if !self.views.contains_key(&view_id) || self.views[&view_id].slots < n.max(1) {
            let slots = n.max(16).next_power_of_two();
            let (uniforms, uniform_group) = self.uniform_buffer(device, slots);
            self.views.insert(
                view_id,
                ViewImages {
                    uniforms,
                    uniform_group,
                    slots,
                    vertices: None,
                    keys: Vec::new(),
                },
            );
        }
        let stride = self.stride as usize;
        let mut data = vec![0u8; stride * n.max(1)];
        let mut vertices: Vec<Vertex> = Vec::with_capacity(n * 6);
        let mut keys = Vec::with_capacity(n);
        for (i, q) in quads.iter().enumerate() {
            let origin = q.corners[0];
            let u = Uniforms {
                mvp: camera.chunk_mvp(origin).to_cols_array_2d(),
                params: [q.opacity.clamp(0.0, 1.0), 0.0, 0.0, 0.0],
            };
            data[i * stride..i * stride + std::mem::size_of::<Uniforms>()]
                .copy_from_slice(bytemuck::bytes_of(&u));
            let rel = |p: DVec3| (p - origin).as_vec3().to_array();
            let [u0, v0, u1, v1] = q.uv;
            let corner = |k: usize, uv: [f32; 2]| Vertex {
                position: rel(q.corners[k]),
                uv,
            };
            let (tl, tr, br, bl) = (
                corner(0, [u0, v0]),
                corner(1, [u1, v0]),
                corner(2, [u1, v1]),
                corner(3, [u0, v1]),
            );
            vertices.extend_from_slice(&[tl, bl, br, tl, br, tr]);
            keys.push(q.key);
        }
        let view = self.views.get_mut(&view_id).expect("view images");
        queue.write_buffer(&view.uniforms, 0, &data);
        view.keys = keys;
        view.vertices = (!vertices.is_empty()).then(|| {
            use wgpu::util::DeviceExt;
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("cad_render_image_vertices"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });
    }

    pub(crate) fn draw(&self, pass: &mut wgpu::RenderPass<'_>, view_id: u64) {
        let Some(view) = self.views.get(&view_id) else {
            return;
        };
        let Some(vertices) = &view.vertices else {
            return;
        };
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, vertices.slice(..));
        for (i, key) in view.keys.iter().enumerate() {
            let Some(tile) = self.tiles.get(key) else {
                continue;
            };
            pass.set_bind_group(0, &view.uniform_group, &[(i as u64 * self.stride) as u32]);
            pass.set_bind_group(1, &tile.bind_group, &[]);
            let first = (i * 6) as u32;
            pass.draw(first..first + 6, 0..1);
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use glam::DVec2;

    use super::*;

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        let mut f = std::pin::pin!(f);
        loop {
            if let std::task::Poll::Ready(v) = f.as_mut().poll(&mut cx) {
                return v;
            }
            std::thread::yield_now();
        }
    }

    /// Draws a 2 × 2 texel tile over a 64 px plan view and reads it back:
    /// each texel lands in its own corner (north up), and opacity blends.
    #[test]
    fn tiles_draw_north_up_and_blend() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let Ok(adapter) = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let (device, queue) =
            block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("device");
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut res = ImageResources::new(&device, format, 1);
        let size = 64u32;
        let camera = Camera {
            distance: 20.0,
            viewport: DVec2::splat(f64::from(size)),
            ..Camera::default()
        };
        // Exactly the view: half-height = distance · tan(fov / 2).
        let half = camera.view_height() * 0.5;
        let quad = |opacity: f32, key: u64| ImageQuad {
            key,
            pixels: Arc::new(ImagePixels {
                width: 2,
                height: 2,
                // Top-left red, top-right green, bottom-left blue, bottom-right white.
                rgba: vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255],
            }),
            corners: [
                DVec3::new(-half, half, 0.0),
                DVec3::new(half, half, 0.0),
                DVec3::new(half, -half, 0.0),
                DVec3::new(-half, -half, 0.0),
            ],
            uv: [0.0, 0.0, 1.0, 1.0],
            opacity,
        };
        let read = |res: &mut ImageResources, frame: &ImageFrame| -> Vec<u8> {
            res.prepare(&device, &queue, 1, &camera, Some(frame));
            let target = |format, usage| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: None,
                    size: wgpu::Extent3d {
                        width: size,
                        height: size,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
            };
            let color = target(
                format,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            );
            let depth = target(DEPTH_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT);
            let (cv, dv) = (
                color.create_view(&Default::default()),
                depth.create_view(&Default::default()),
            );
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &cv,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &dv,
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
                res.draw(&mut pass, 1);
            }
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: u64::from(size * size * 4),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                color.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(size * 4),
                        rows_per_image: Some(size),
                    },
                },
                color.size(),
            );
            queue.submit([encoder.finish()]);
            buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
            device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
            buffer.slice(..).get_mapped_range().expect("mapped").to_vec()
        };
        let px = |data: &[u8], x: u32, y: u32| -> [u8; 4] {
            let i = ((y * size + x) * 4) as usize;
            [data[i], data[i + 1], data[i + 2], data[i + 3]]
        };
        let data = read(
            &mut res,
            &ImageFrame {
                quads: vec![quad(1.0, 1)],
            },
        );
        // Quarter points are near texel centres (a little bilinear blending).
        let near =
            |p: [u8; 4], want: [u8; 3]| (0..3).all(|k| (i32::from(p[k]) - i32::from(want[k])).abs() <= 12);
        assert!(
            near(px(&data, 16, 16), [255, 0, 0]),
            "north-west is the top-left texel: {:?}",
            px(&data, 16, 16)
        );
        assert!(near(px(&data, 48, 16), [0, 255, 0]), "{:?}", px(&data, 48, 16));
        assert!(near(px(&data, 16, 48), [0, 0, 255]), "{:?}", px(&data, 16, 48));
        assert!(
            near(px(&data, 48, 48), [255, 255, 255]),
            "{:?}",
            px(&data, 48, 48)
        );
        // Half opacity over black: half the colour.
        let data = read(
            &mut res,
            &ImageFrame {
                quads: vec![quad(0.5, 1)],
            },
        );
        let p = px(&data, 16, 16);
        assert!((i32::from(p[0]) - 124).abs() <= 8 && p[1] <= 6, "{p:?}");
        // A sub-rectangle of the tile (as a coarser tile standing in for a finer one).
        let mut q = quad(1.0, 1);
        q.uv = [0.5, 0.5, 1.0, 1.0];
        let data = read(&mut res, &ImageFrame { quads: vec![q] });
        assert!(
            near(px(&data, 32, 32), [255, 255, 255]),
            "{:?}",
            px(&data, 32, 32)
        );
        assert!(block_on(scope.pop()).is_none());
    }
}
