//! Out-of-core point clouds, drawn node by node from an octree, as Potree
//! does. The host chooses the nodes for the view (see
//! `survey_io::copc::CopcSource::select`) and hands them over as a
//! [`CloudFrame`]; the renderer keeps each node's points in a GPU buffer of
//! its own (uploaded once, evicted least recently drawn first), so moving the
//! view only uploads nodes it has not seen. Points are sized to the spacing of
//! the deepest drawn node where they lie (adaptive point size), and the pass
//! is composited with eye-dome lighting.
//!
//! Native only: the shaders read a storage buffer in the vertex stage, which
//! WebGL2 lacks (and COPC reading is native only).

use std::sync::Arc;

use glam::DVec3;

/// One octree node to draw.
#[derive(Clone)]
pub struct CloudNode {
    /// Identifies the node's content across frames (keys the GPU cache).
    pub id: u64,
    /// Centre of the node's cube; points are offsets from it.
    pub center: DVec3,
    /// Edge length of the node's cube.
    pub side: f64,
    /// Plan spacing between points where this is the deepest drawn node.
    pub spacing: f32,
    /// Index of the parent in [`CloudLayer::nodes`] (parents come first).
    pub parent: Option<u32>,
    /// Which child of the parent: x | y << 1 | z << 2 (1 = the high half).
    pub octant: u8,
    /// Offsets from `center` with the RGBA8 colour's bits in `w`; `None`
    /// while the node is still loading (it and its subtree are not drawn).
    /// The alpha byte is the point's local plan density relative to the
    /// node's average, `128 + 16·log2(ratio)` (128 when even): points are
    /// sized to their local spacing, not the node's average.
    pub points: Option<Arc<Vec<[f32; 4]>>>,
    /// The points grouped by sub-cell of the node's cube ([`CELLS_PER_SIDE`]
    /// per side, index x + n·y + n²·z): cell `i` is `points[cells[i]..cells[i + 1]]`,
    /// shuffled so any prefix is an even sample. Cells outside the view are
    /// skipped. `None` draws the points as one run.
    pub cells: Option<Arc<[u32; CELLS + 1]>>,
    /// Share of each cell to draw (its prefix); 1 for all.
    pub fraction: f32,
}

/// Sub-cells per side of a node's cube (see [`CloudNode::cells`]).
pub const CELLS_PER_SIDE: usize = 4;
/// Sub-cells per node.
pub const CELLS: usize = CELLS_PER_SIDE * CELLS_PER_SIDE * CELLS_PER_SIDE;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CloudColoring {
    /// The points' own colours.
    Source,
    /// The height ramp between these elevations.
    Height { min: f64, max: f64 },
    /// One colour (RGBA8).
    Single([u8; 4]),
}

/// One cloud's nodes and style.
#[derive(Clone)]
pub struct CloudLayer {
    pub nodes: Vec<CloudNode>,
    pub coloring: CloudColoring,
    /// Smallest drawn point size (logical px).
    pub min_size_px: f32,
}

/// The out-of-core clouds of one viewport.
#[derive(Clone, Default)]
pub struct CloudFrame {
    pub layers: Vec<CloudLayer>,
    /// Eye-dome lighting strength (0 = off).
    pub edl: f32,
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use gpu::{CloudResources, CloudView};

#[cfg(not(target_arch = "wasm32"))]
mod gpu {
    use std::collections::HashMap;
    use std::num::NonZeroU64;

    use bytemuck::{Pod, Zeroable};
    use egui_wgpu::wgpu::{self, util::DeviceExt as _};

    use super::{CELLS, CELLS_PER_SIDE, CloudColoring, CloudFrame};
    use crate::camera::{Camera, Projection};

    /// Largest drawn point size (logical px). Potree allows 50; a lower cap
    /// keeps a coarse level drawn near the camera (while finer nodes load)
    /// from turning into large blocks, and small gaps are filled in screen
    /// space instead (see `cloud_edl.wgsl`).
    const MAX_SIZE_PX: f32 = 10.0;
    /// Points are drawn this many spacings across, so randomly placed points
    /// overlap rather than leave gaps.
    const FILL: f32 = 1.5;
    /// Most points uploaded in one frame; the rest follow over the next frames.
    const UPLOAD_PER_FRAME: usize = 2_000_000;
    /// Points kept on the GPU per viewport before nodes not drawn this frame
    /// are evicted, least recently drawn first.
    const GPU_POINTS: usize = 16_000_000;
    /// Eye-dome lighting neighbour distance (logical px), as Potree's default.
    const EDL_RADIUS_PX: f32 = 1.4;
    /// Widest hole between points filled in screen space (logical px).
    const FILL_HOLES_PX: f32 = 3.0;
    const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
    const LINEAR_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
    const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

    #[derive(Clone, Copy, Pod, Zeroable)]
    #[repr(C)]
    struct Draw {
        mvp: [[f32; 4]; 4],
        frame: [f32; 4],
        node: [f32; 4],
        depth: [f32; 4],
        color: [f32; 4],
        single: [f32; 4],
    }

    pub(crate) struct CloudResources {
        draw_layout: wgpu::BindGroupLayout,
        edl_layout: wgpu::BindGroupLayout,
        pipeline: wgpu::RenderPipeline,
        composite: wgpu::RenderPipeline,
        stride: u64,
    }

    impl CloudResources {
        /// `scene_format`, `samples`: the main pass the composite draws into.
        pub(crate) fn new(device: &wgpu::Device, scene_format: wgpu::TextureFormat, samples: u32) -> Self {
            let draw_size = std::mem::size_of::<Draw>() as u64;
            let align = u64::from(device.limits().min_uniform_buffer_offset_alignment);
            let stride = draw_size.div_ceil(align) * align;
            let draw_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("cad_render_cloud_draw"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: true,
                            min_binding_size: NonZeroU64::new(draw_size),
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: NonZeroU64::new(16),
                        },
                        count: None,
                    },
                ],
            });
            let texture = |binding, sample_type| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            };
            let edl_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("cad_render_cloud_edl"),
                entries: &[
                    texture(0, wgpu::TextureSampleType::Float { filterable: false }),
                    texture(1, wgpu::TextureSampleType::Float { filterable: false }),
                    texture(2, wgpu::TextureSampleType::Depth),
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: NonZeroU64::new(16),
                        },
                        count: None,
                    },
                ],
            });

            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("cad_render_clouds"),
                source: wgpu::ShaderSource::Wgsl(include_str!("shaders/clouds.wgsl").into()),
            });
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("cad_render_clouds"),
                bind_group_layouts: &[Some(&draw_layout)],
                immediate_size: 0,
            });
            let attrs = wgpu::vertex_attr_array![0 => Float32x4];
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("cad_render_clouds"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_cloud"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: 16,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &attrs,
                    })],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_cloud"),
                    compilation_options: Default::default(),
                    targets: &[
                        Some(wgpu::ColorTargetState {
                            format: COLOR_FORMAT,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        }),
                        Some(wgpu::ColorTargetState {
                            format: LINEAR_DEPTH_FORMAT,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        }),
                    ],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(true),
                    // Reverse-Z: larger depth is closer.
                    depth_compare: Some(wgpu::CompareFunction::Greater),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });

            let edl_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("cad_render_cloud_edl"),
                source: wgpu::ShaderSource::Wgsl(include_str!("shaders/cloud_edl.wgsl").into()),
            });
            let edl_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("cad_render_cloud_edl"),
                bind_group_layouts: &[Some(&edl_layout)],
                immediate_size: 0,
            });
            let composite = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("cad_render_cloud_edl"),
                layout: Some(&edl_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &edl_shader,
                    entry_point: Some("vs_edl"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &edl_shader,
                    entry_point: Some("fs_edl"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: scene_format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: crate::renderer::DEPTH_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Always),
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
            Self {
                draw_layout,
                edl_layout,
                pipeline,
                composite,
                stride,
            }
        }
    }

    struct GpuNode {
        buffer: wgpu::Buffer,
        count: u32,
        last_used: u64,
    }

    struct Targets {
        size: [u32; 2],
        color: wgpu::TextureView,
        linear: wgpu::TextureView,
        depth: wgpu::TextureView,
        edl_group: wgpu::BindGroup,
    }

    /// One viewport's cloud state: the GPU node cache (kept across resizes)
    /// and the offscreen targets of the cloud pass.
    #[derive(Default)]
    pub(crate) struct CloudView {
        nodes: HashMap<u64, GpuNode>,
        resident: usize,
        frame: u64,
        targets: Option<Targets>,
        edl_buffer: Option<wgpu::Buffer>,
        tree: Option<(wgpu::Buffer, usize)>,
        draws: Option<(wgpu::Buffer, usize)>,
        group: Option<wgpu::BindGroup>,
        /// Whether the last prepared frame drew anything to composite.
        drew: bool,
    }

    impl CloudView {
        /// Uploads what the frame needs, builds the drawn octree and renders
        /// the cloud pass into its targets. Returns true while nodes are still
        /// to be uploaded (the host should draw another frame).
        #[allow(clippy::too_many_arguments)]
        pub(crate) fn prepare(
            &mut self,
            res: &CloudResources,
            device: &wgpu::Device,
            queue: &wgpu::Queue,
            encoder: &mut wgpu::CommandEncoder,
            camera: &Camera,
            frame: &CloudFrame,
            size: [u32; 2],
            ppp: f32,
        ) -> bool {
            self.frame += 1;
            let now = self.frame;
            self.drew = false;

            // Upload missing nodes in the given order (parents first), within
            // the frame's budget.
            let mut uploaded = 0usize;
            let mut pending = false;
            for layer in &frame.layers {
                for n in &layer.nodes {
                    let Some(points) = &n.points else {
                        pending = true;
                        continue;
                    };
                    if points.is_empty() || self.nodes.contains_key(&n.id) {
                        continue;
                    }
                    if uploaded >= UPLOAD_PER_FRAME {
                        pending = true;
                        continue;
                    }
                    let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("cad_render_cloud_node"),
                        contents: bytemuck::cast_slice(points.as_slice()),
                        usage: wgpu::BufferUsages::VERTEX,
                    });
                    self.nodes.insert(
                        n.id,
                        GpuNode {
                            buffer,
                            count: points.len() as u32,
                            last_used: 0,
                        },
                    );
                    uploaded += points.len();
                    self.resident += points.len();
                }
            }

            // The drawn octree: nodes that are loaded and whose parent is drawn,
            // laid out so each node's drawn children are contiguous.
            let mut tree: Vec<[u32; 4]> = Vec::new();
            // (layer, node index, tree index) of each node with points to draw.
            let mut draws: Vec<(usize, usize, u32)> = Vec::new();
            for (li, layer) in frame.layers.iter().enumerate() {
                let n = layer.nodes.len();
                let mut drawn = vec![false; n];
                let mut children: Vec<Vec<usize>> = vec![Vec::new(); n];
                let mut roots = Vec::new();
                for (i, node) in layer.nodes.iter().enumerate() {
                    let loaded = node
                        .points
                        .as_ref()
                        .is_some_and(|p| p.is_empty() || self.nodes.contains_key(&node.id));
                    let parent_drawn = node
                        .parent
                        .is_none_or(|p| drawn.get(p as usize).copied().unwrap_or(false));
                    drawn[i] = loaded && parent_drawn;
                    if !drawn[i] {
                        continue;
                    }
                    match node.parent {
                        Some(p) => children[p as usize].push(i),
                        None => roots.push(i),
                    }
                }
                // Breadth first: a node's children get consecutive slots.
                let mut slot = vec![0u32; n];
                let mut queue: std::collections::VecDeque<usize> = roots.iter().copied().collect();
                for &r in &roots {
                    slot[r] = tree.len() as u32;
                    tree.push([0; 4]);
                }
                while let Some(i) = queue.pop_front() {
                    let node = &layer.nodes[i];
                    let mut kids = children[i].clone();
                    kids.sort_by_key(|&c| layer.nodes[c].octant);
                    let mut mask = 0u32;
                    let first = tree.len() as u32;
                    for &c in &kids {
                        mask |= 1 << layer.nodes[c].octant;
                        slot[c] = tree.len() as u32;
                        tree.push([0; 4]);
                        queue.push_back(c);
                    }
                    tree[slot[i] as usize] = [mask, first, node.spacing.to_bits(), node.fraction.to_bits()];
                    if node.points.as_ref().is_some_and(|p| !p.is_empty()) {
                        draws.push((li, i, slot[i]));
                    }
                }
            }

            // Evict nodes not drawn this frame, least recently drawn first.
            for &(li, i, _) in &draws {
                if let Some(g) = self.nodes.get_mut(&frame.layers[li].nodes[i].id) {
                    g.last_used = now;
                }
            }
            if self.resident > GPU_POINTS {
                let mut idle: Vec<(u64, u64)> = self
                    .nodes
                    .iter()
                    .filter(|(_, g)| g.last_used < now)
                    .map(|(id, g)| (g.last_used, *id))
                    .collect();
                idle.sort_unstable();
                for (_, id) in idle {
                    if self.resident <= GPU_POINTS {
                        break;
                    }
                    if let Some(g) = self.nodes.remove(&id) {
                        self.resident -= g.count as usize;
                    }
                }
            }
            if draws.is_empty() {
                return pending;
            }

            self.ensure_targets(res, device, size);
            // Grow the shared buffers as needed (the bind group follows them).
            let mut rebind = false;
            if self.tree.as_ref().is_none_or(|(_, cap)| *cap < tree.len()) {
                let cap = tree.len().next_power_of_two();
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("cad_render_cloud_tree"),
                    size: (cap * 16) as u64,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                self.tree = Some((buffer, cap));
                rebind = true;
            }
            if self.draws.as_ref().is_none_or(|(_, cap)| *cap < draws.len()) {
                let cap = draws.len().next_power_of_two();
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("cad_render_cloud_draws"),
                    size: cap as u64 * res.stride,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                self.draws = Some((buffer, cap));
                rebind = true;
            }
            let (Some((tree_buffer, _)), Some((draw_buffer, _))) = (&self.tree, &self.draws) else {
                return pending;
            };
            if rebind || self.group.is_none() {
                self.group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("cad_render_cloud_draw"),
                    layout: &res.draw_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                buffer: draw_buffer,
                                offset: 0,
                                size: NonZeroU64::new(std::mem::size_of::<Draw>() as u64),
                            }),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: tree_buffer.as_entire_binding(),
                        },
                    ],
                }));
            }
            queue.write_buffer(tree_buffer, 0, bytemuck::cast_slice(&tree));

            // Physical px per world unit: at unit depth in perspective (divided
            // by the depth in the shader), everywhere in orthographic.
            let px_per_world = match camera.projection {
                Projection::Perspective => f64::from(size[1]) / (2.0 * (camera.fov_y * 0.5).tan()),
                Projection::Orthographic => f64::from(ppp) / camera.world_per_pixel().max(1e-12),
            } as f32;
            let (eye, forward) = (camera.eye(), camera.forward());
            let mut data = vec![0u8; draws.len() * res.stride as usize];
            for (k, &(li, i, slot)) in draws.iter().enumerate() {
                let layer = &frame.layers[li];
                let node = &layer.nodes[i];
                let (mode, z_rel, inv_span, single) = match layer.coloring {
                    CloudColoring::Source => (0.0, 0.0, 0.0, [0.0; 4]),
                    CloudColoring::Height { min, max } => (
                        1.0,
                        (node.center.z - min) as f32,
                        (1.0 / (max - min).max(1e-9)) as f32,
                        [0.0; 4],
                    ),
                    CloudColoring::Single(c) => (2.0, 0.0, 0.0, c.map(|v| f32::from(v) / 255.0)),
                };
                let draw = Draw {
                    mvp: camera.chunk_mvp(node.center).to_cols_array_2d(),
                    frame: [size[0] as f32, size[1] as f32, px_per_world, FILL],
                    node: [
                        f32::from_bits(slot),
                        node.side as f32,
                        layer.min_size_px * ppp,
                        MAX_SIZE_PX * ppp,
                    ],
                    depth: [
                        forward.x as f32,
                        forward.y as f32,
                        forward.z as f32,
                        forward.dot(node.center - eye) as f32,
                    ],
                    color: [mode, z_rel, inv_span, 0.0],
                    single,
                };
                let at = k * res.stride as usize;
                data[at..at + std::mem::size_of::<Draw>()].copy_from_slice(bytemuck::bytes_of(&draw));
            }
            queue.write_buffer(draw_buffer, 0, &data);
            if let Some(edl) = &self.edl_buffer {
                queue.write_buffer(
                    edl,
                    0,
                    bytemuck::cast_slice(&[
                        frame.edl,
                        EDL_RADIUS_PX * ppp,
                        (FILL_HOLES_PX * ppp).round(),
                        0.0,
                    ]),
                );
            }

            // Instance ranges per draw: the drawn share of each cell in view.
            let frustum = camera.rect_frustum(glam::DVec2::ZERO, camera.viewport);
            let ranges: Vec<Vec<std::ops::Range<u32>>> = draws
                .iter()
                .map(|&(li, i, _)| {
                    let node = &frame.layers[li].nodes[i];
                    let len = node.points.as_ref().map_or(0, |p| p.len() as u32);
                    let share = |n: u32, fraction: f32| ((n as f32 * fraction).ceil() as u32).min(n);
                    let Some(cells) = &node.cells else {
                        return std::iter::once(0..share(len, node.fraction)).collect();
                    };
                    // A node drawn in part takes more of its sparse columns and
                    // less of its dense ones (all of a column at most), so a node
                    // mixing woodland and field reaches the same spacing over
                    // both. Full nodes draw everything. The selection counts the
                    // same shares: keep in step with `survey_io::copc::drawn_share`.
                    let per_side = CELLS_PER_SIDE;
                    let mut columns = [0u32; CELLS_PER_SIDE * CELLS_PER_SIDE];
                    for c in 0..CELLS {
                        columns[c % (per_side * per_side)] += cells[c + 1] - cells[c];
                    }
                    let column_share = |c: usize| {
                        if node.fraction >= 1.0 || len == 0 {
                            return node.fraction;
                        }
                        let ratio = columns[c % (per_side * per_side)] as f32 * (per_side * per_side) as f32
                            / len as f32;
                        (node.fraction / ratio.max(1e-6)).min(1.0)
                    };
                    let cell = node.side / CELLS_PER_SIDE as f64;
                    let min = node.center - glam::DVec3::splat(node.side * 0.5);
                    (0..CELLS)
                        .filter_map(|c| {
                            let (start, end) = (cells[c], cells[c + 1]);
                            if end <= start {
                                return None;
                            }
                            let at = glam::DVec3::new(
                                (c % CELLS_PER_SIDE) as f64,
                                (c / CELLS_PER_SIDE % CELLS_PER_SIDE) as f64,
                                (c / (CELLS_PER_SIDE * CELLS_PER_SIDE)) as f64,
                            );
                            let lo = min + at * cell;
                            let bounds = survey_core::Aabb3::new(lo, lo + glam::DVec3::splat(cell));
                            frustum
                                .intersects_aabb(&bounds)
                                .then(|| start..start + share(end - start, column_share(c)))
                        })
                        .collect()
                })
                .collect();
            let (Some(targets), Some(group)) = (&self.targets, &self.group) else {
                return pending;
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cad_render_clouds"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: &targets.color,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: &targets.linear,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&res.pipeline);
            for (k, &(li, i, _)) in draws.iter().enumerate() {
                let Some(g) = self.nodes.get(&frame.layers[li].nodes[i].id) else {
                    continue;
                };
                if ranges[k].is_empty() {
                    continue;
                }
                pass.set_bind_group(0, group, &[(k as u64 * res.stride) as u32]);
                pass.set_vertex_buffer(0, g.buffer.slice(..));
                for r in &ranges[k] {
                    pass.draw(0..6, r.start.min(g.count)..r.end.min(g.count));
                }
            }
            drop(pass);
            self.drew = true;
            pending
        }

        #[cfg(test)]
        pub(crate) fn drew(&self) -> bool {
            self.drew
        }

        /// Draws the cloud pass into the main pass (call first in it).
        pub(crate) fn composite<'a>(&'a self, res: &'a CloudResources, pass: &mut wgpu::RenderPass<'a>) {
            if !self.drew {
                return;
            }
            let Some(targets) = &self.targets else { return };
            pass.set_pipeline(&res.composite);
            pass.set_bind_group(0, &targets.edl_group, &[]);
            pass.draw(0..3, 0..1);
        }

        fn ensure_targets(&mut self, res: &CloudResources, device: &wgpu::Device, size: [u32; 2]) {
            if self.targets.as_ref().is_some_and(|t| t.size == size) {
                return;
            }
            let edl_buffer = self
                .edl_buffer
                .get_or_insert_with(|| {
                    device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("cad_render_cloud_edl"),
                        size: 16,
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    })
                })
                .clone();
            let make = |label, format| {
                device
                    .create_texture(&wgpu::TextureDescriptor {
                        label: Some(label),
                        size: wgpu::Extent3d {
                            width: size[0],
                            height: size[1],
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    })
                    .create_view(&wgpu::TextureViewDescriptor::default())
            };
            let color = make("cad_render_cloud_color", COLOR_FORMAT);
            let linear = make("cad_render_cloud_linear_depth", LINEAR_DEPTH_FORMAT);
            let depth = make("cad_render_cloud_depth", DEPTH_FORMAT);
            let edl_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("cad_render_cloud_edl"),
                layout: &res.edl_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&color),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&linear),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&depth),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: edl_buffer.as_entire_binding(),
                    },
                ],
            });
            self.targets = Some(Targets {
                size,
                color,
                linear,
                depth,
                edl_group,
            });
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::sync::Arc;

    use egui_wgpu::wgpu;
    use glam::{DVec2, DVec3};

    use super::*;
    use crate::camera::{Camera, Projection};

    /// Polls a future to completion (wgpu's native futures resolve promptly).
    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        let waker = std::task::Waker::noop();
        let mut cx = std::task::Context::from_waker(waker);
        let mut f = std::pin::pin!(f);
        loop {
            if let std::task::Poll::Ready(v) = f.as_mut().poll(&mut cx) {
                return v;
            }
            std::thread::yield_now();
        }
    }

    fn grid(n: usize, step: f32) -> Arc<Vec<[f32; 4]>> {
        let c = f32::from_bits(u32::from_le_bytes([200, 100, 50, 128]));
        let h = n as f32 * step * 0.5;
        Arc::new(
            (0..n * n)
                .map(|i| [(i % n) as f32 * step - h, (i / n) as f32 * step - h, 0.0, c])
                .collect(),
        )
    }

    /// The cloud pass and its composite run without validation errors on a
    /// real device (skipped where there is no adapter).
    #[test]
    fn cloud_pass_and_composite_validate() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let Ok(adapter) = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
            eprintln!("no GPU adapter: skipped");
            return;
        };
        let (device, queue) =
            block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("device");
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);

        let format = wgpu::TextureFormat::Rgba8Unorm;
        let samples = 4;
        let res = gpu::CloudResources::new(&device, format, samples);
        let mut view = CloudView::default();
        // A root with one drawn child (octant 0).
        let frame = CloudFrame {
            layers: vec![CloudLayer {
                nodes: vec![
                    CloudNode {
                        id: 1,
                        center: DVec3::ZERO,
                        side: 100.0,
                        spacing: 2.0,
                        parent: None,
                        octant: 0,
                        points: Some(grid(50, 2.0)),
                        cells: None,
                        fraction: 1.0,
                    },
                    CloudNode {
                        id: 2,
                        center: DVec3::splat(-25.0),
                        side: 50.0,
                        spacing: 1.0,
                        parent: Some(0),
                        octant: 0,
                        points: Some(grid(50, 1.0)),
                        cells: None,
                        fraction: 0.5,
                    },
                ],
                coloring: CloudColoring::Source,
                min_size_px: 2.0,
            }],
            edl: 1.0,
        };
        let size = [320u32, 200];
        let mut camera = Camera {
            target: DVec3::ZERO,
            distance: 150.0,
            pitch: 0.6,
            projection: Projection::Perspective,
            viewport: DVec2::new(320.0, 200.0),
            ..Camera::default()
        };
        camera.fit_clip(&survey_core::Aabb3::new(DVec3::splat(-60.0), DVec3::splat(60.0)));

        let mut encoder = device.create_command_encoder(&Default::default());
        view.prepare(&res, &device, &queue, &mut encoder, &camera, &frame, size, 1.0);
        let target = |format, usage| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: None,
                    size: wgpu::Extent3d {
                        width: size[0],
                        height: size[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let color = target(format, wgpu::TextureUsages::RENDER_ATTACHMENT);
        let depth = target(
            crate::renderer::DEPTH_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth,
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
            view.composite(&res, &mut pass);
        }
        queue.submit([encoder.finish()]);
        let error = block_on(scope.pop());
        assert!(error.is_none(), "{error:?}");
        assert!(view.drew(), "nothing was drawn");
    }
}

/// Renders a real COPC file through the cloud pass and composite, offscreen,
/// to PNGs for eyeballing: `CLOUD_COPC=<file> CLOUD_OUT=<dir> cargo test -p
/// cad-render render_copc -- --ignored`.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod visual {
    use std::sync::Arc;

    use egui_wgpu::wgpu;
    use glam::{DVec2, DVec3};

    use super::*;
    use crate::camera::{Camera, Projection};

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

    #[test]
    #[ignore]
    fn render_copc() {
        let path = std::env::var("CLOUD_COPC").expect("CLOUD_COPC");
        let out = std::path::PathBuf::from(std::env::var("CLOUD_OUT").expect("CLOUD_OUT"));
        let src = survey_io::copc::CopcSource::shared(std::path::Path::new(&path)).expect("open");
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter =
            block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("adapter");
        let (device, queue) =
            block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("device");
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let res = gpu::CloudResources::new(&device, format, 1);
        let ppp = 2.0f32;
        let vp = DVec2::new(1000.0, 530.0);
        let size = [(vp.x * f64::from(ppp)) as u32, (vp.y * f64::from(ppp)) as u32];
        let centre = DVec3::new(659893.79, 6018785.5, 160.0);
        let views = [
            (
                "plan",
                std::f64::consts::FRAC_PI_2,
                700.0,
                Projection::Orthographic,
            ),
            ("oblique30", 30f64.to_radians(), 150.0, Projection::Perspective),
            ("farm", 50f64.to_radians(), 120.0, Projection::Perspective),
            ("close", 35f64.to_radians(), 25.0, Projection::Perspective),
            ("mid", 70f64.to_radians(), 300.0, Projection::Perspective),
            // Low across the field to the farm, and the same view drawn from
            // the selection for a camera further back (the app draws the last
            // selection until a new one arrives).
            ("low", 28f64.to_radians(), 130.0, Projection::Perspective),
            ("stale", 28f64.to_radians(), 110.0, Projection::Perspective),
        ];
        // The farmhouse, picked in the plan view.
        let farm = Camera {
            target: centre,
            distance: 700.0,
            yaw: 0.6,
            projection: Projection::Orthographic,
            viewport: vp,
            ..Camera::default()
        }
        .screen_to_target_plane(DVec2::new(300.0, 345.0));
        for (name, pitch, distance, projection) in views {
            let yaw = match name {
                "low" | "stale" => 2.0 * std::f64::consts::PI / 3.0,
                _ => 0.6,
            };
            let target = if matches!(name, "farm" | "low" | "stale") {
                DVec3::new(farm.x, farm.y, 150.0)
            } else {
                centre
            };
            let mut camera = Camera {
                target,
                distance,
                pitch,
                yaw,
                projection,
                viewport: vp,
                ..Camera::default()
            };
            camera.fit_clip(&survey_core::Aabb3::new(
                DVec3::new(659473.0, 6018407.0, 116.0),
                DVec3::new(660315.0, 6019164.0, 282.0),
            ));
            let margin = vp * 0.25;
            let mut chooser = camera.clone();
            if name == "stale" {
                chooser.distance = 200.0;
            }
            let view = survey_io::copc::ViewQuery {
                frustum: chooser.rect_frustum(-margin, vp + margin).planes().to_vec(),
                world_per_pixel: chooser.world_per_pixel(),
                eye: (projection == Projection::Perspective).then(|| (chooser.eye(), chooser.distance)),
                spacing_px: 2.0,
            };
            let t = std::time::Instant::now();
            let selection = src.select(&view, 3_000_000);
            let keys: Vec<_> = selection.iter().map(|s| s.key).collect();
            src.load(&keys).expect("load");
            let total: u32 = selection.iter().map(|s| s.count).sum();
            eprintln!(
                "{name}: {} nodes, {total} points, selected+loaded in {:?}",
                selection.len(),
                t.elapsed()
            );
            let mut levels = std::collections::BTreeMap::<i32, (usize, f64, usize)>::new();
            for sel in &selection {
                let e = levels.entry(sel.key.0).or_default();
                e.0 += 1;
                e.1 += f64::from(sel.count) * f64::from(sel.fraction);
                if sel.fraction >= 1.0 {
                    e.2 += 1;
                }
            }
            for (l, (n, pts, full)) in levels {
                eprintln!("   level {l}: {n} nodes ({full} full), {pts:.0} drawn pts (before cell culling)");
            }
            let nodes = selection
                .iter()
                .map(|s| CloudNode {
                    id: (s.key.0 as u64) << 48
                        ^ (s.key.1 as u64) << 32
                        ^ (s.key.2 as u64) << 16
                        ^ s.key.3 as u64,
                    center: s.center,
                    side: s.side,
                    spacing: s.spacing as f32,
                    parent: s.parent.map(|p| p as u32),
                    octant: s.octant,
                    points: if s.count == 0 {
                        Some(Arc::new(Vec::new()))
                    } else {
                        src.cached(s.key).map(|n| n.points.clone())
                    },
                    cells: src.cached(s.key).map(|n| n.cells.clone()),
                    fraction: s.fraction,
                })
                .collect();
            let frame = CloudFrame {
                layers: vec![CloudLayer {
                    nodes,
                    coloring: CloudColoring::Source,
                    // survey_model's OUT_OF_CORE_POINT_SIZE.
                    min_size_px: 7.0,
                }],
                edl: std::env::var("CLOUD_EDL")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1.0),
            };
            let mut cloud_view = CloudView::default();
            for _ in 0..8 {
                let mut encoder = device.create_command_encoder(&Default::default());
                let pending =
                    cloud_view.prepare(&res, &device, &queue, &mut encoder, &camera, &frame, size, ppp);
                queue.submit([encoder.finish()]);
                if !pending {
                    break;
                }
            }
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let depth = device
                .create_texture(&wgpu::TextureDescriptor {
                    label: None,
                    size: texture.size(),
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: crate::renderer::DEPTH_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&Default::default());
            let color = texture.create_view(&Default::default());
            let row = (size[0] * 4).div_ceil(256) * 256;
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: u64::from(row * size[1]),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: None,
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &color,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth,
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
                cloud_view.composite(&res, &mut pass);
            }
            encoder.copy_texture_to_buffer(
                texture.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(row),
                        rows_per_image: None,
                    },
                },
                texture.size(),
            );
            queue.submit([encoder.finish()]);
            readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
            device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
            let data = readback.slice(..).get_mapped_range().expect("mapped");
            let mut pixels = Vec::with_capacity((size[0] * size[1] * 4) as usize);
            for y in 0..size[1] {
                let start = (y * row) as usize;
                pixels.extend_from_slice(&data[start..start + (size[0] * 4) as usize]);
            }
            let file = std::fs::File::create(out.join(format!("{name}.png"))).expect("png file");
            let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), size[0], size[1]);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .expect("png header")
                .write_image_data(&pixels)
                .expect("png data");
        }
    }
}
