// Out-of-core point clouds, one draw per octree node (see `clouds.rs`).
// Points are f32 offsets from the node's centre with their RGBA8 colour's
// bits in w. Each point is a screen-aligned square sized, as Potree's
// adaptive mode does, to the spacing of the deepest node drawn where it lies:
// coarse areas get larger points and refined areas smaller ones, so neither
// shows holes. Colour and linear view depth go to two targets for the
// eye-dome lighting composite (`cloud_edl.wgsl`). Depth is reverse-Z.

struct Draw {
    mvp: mat4x4<f32>,
    // x, y: target size (physical px); z: physical px per world unit at unit
    // depth (perspective) or per world unit (orthographic); w: fill factor.
    frame: vec4<f32>,
    // x: the node's index in `tree` (u32 bits); y: node edge length;
    // z, w: smallest and largest point size (physical px).
    node: vec4<f32>,
    // View depth as dot(xyz, offset) + w.
    depth: vec4<f32>,
    // x: colour mode (0 source, 1 height, 2 single); y: node centre z minus
    // the height ramp's bottom; z: 1 / ramp span.
    color: vec4<f32>,
    single: vec4<f32>,
};

@group(0) @binding(0) var<uniform> d: Draw;
// The drawn octree: x child mask (bit = octant drawn), y first drawn child
// (children contiguous in octant order), z spacing and w share drawn (f32
// bits).
@group(0) @binding(1) var<storage, read> tree: array<vec4<u32>>;

struct Out {
    @builtin(position) pos: vec4<f32>,
    @location(0) @interpolate(flat) color: vec4<f32>,
    @location(1) @interpolate(flat) depth: f32,
};

// Hypsometric ramp (low → high), as `surface_view::RAMP`.
fn ramp(t: f32) -> vec3<f32> {
    var stops = array<vec3<f32>, 6>(
        vec3<f32>(46.0, 110.0, 64.0),
        vec3<f32>(96.0, 160.0, 82.0),
        vec3<f32>(190.0, 196.0, 110.0),
        vec3<f32>(205.0, 160.0, 92.0),
        vec3<f32>(160.0, 112.0, 80.0),
        vec3<f32>(235.0, 230.0, 220.0),
    );
    let x = clamp(t, 0.0, 1.0) * 5.0;
    let i = min(u32(floor(x)), 4u);
    return mix(stops[i], stops[i + 1u], x - f32(i)) / 255.0;
}

@vertex
fn vs_cloud(@builtin(vertex_index) vi: u32, @location(0) p: vec4<f32>) -> Out {
    var out: Out;
    let clip = d.mvp * vec4<f32>(p.xyz, 1.0);

    // Walk down the drawn octree to the deepest node holding this point.
    var t = bitcast<u32>(d.node.x);
    var local = p.xyz;
    var half = d.node.y * 0.5;
    for (var level = 0u; level < 24u; level++) {
        let n = tree[t];
        let hi = local >= vec3<f32>(0.0);
        let octant = select(0u, 1u, hi.x) | select(0u, 2u, hi.y) | select(0u, 4u, hi.z);
        if ((n.x >> octant) & 1u) == 0u {
            break;
        }
        t = n.y + countOneBits(n.x & ((1u << octant) - 1u));
        let q = half * 0.5;
        local -= select(vec3<f32>(-q), vec3<f32>(q), hi);
        half = q;
    }
    // The node's average spacing, scaled to where this point lies: its alpha
    // byte holds its column's density relative to the node's, 128 + 16·log2.
    // A node drawn in part draws its columns evenly (see `clouds.rs`) except
    // those too sparse to reach its share; a full node draws them as they are.
    // Limited (2x larger, 4x smaller) so a few stray points in a sparse
    // column don't become large blocks.
    let ratio = exp2((f32(bitcast<u32>(p.w) >> 24u) - 128.0) / 16.0);
    let share = bitcast<f32>(tree[t].w);
    var relative = 1.0 / ratio;
    if share < 1.0 {
        relative = max(share / ratio, 1.0);
    }
    let spacing = bitcast<f32>(tree[t].z) * clamp(sqrt(relative), 0.25, 2.0);
    let size = clamp(spacing * d.frame.w * d.frame.z / max(clip.w, 1e-6), d.node.z, d.node.w);

    // Two triangles per point (instanced quad).
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let corner = corners[vi % 6u];
    out.pos = vec4<f32>(clip.xy + corner * size / d.frame.xy * clip.w, clip.z, clip.w);

    let mode = u32(d.color.x);
    if mode == 1u {
        out.color = vec4<f32>(ramp((d.color.y + p.z) * d.color.z), 1.0);
    } else if mode == 2u {
        out.color = d.single;
    } else {
        out.color = unpack4x8unorm(bitcast<u32>(p.w));
    }
    out.depth = dot(d.depth.xyz, p.xyz) + d.depth.w;
    return out;
}

struct Targets {
    @location(0) color: vec4<f32>,
    @location(1) depth: f32,
};

@fragment
fn fs_cloud(in: Out) -> Targets {
    var out: Targets;
    out.color = vec4<f32>(in.color.rgb, 1.0);
    out.depth = max(in.depth, 1e-6);
    return out;
}
