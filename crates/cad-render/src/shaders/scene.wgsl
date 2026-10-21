// Scene shaders. Positions arrive as f32 offsets from a chunk origin; `u.mvp`
// already contains the (f64-computed) origin − eye translation, so no large
// values ever reach the GPU. Depth is reverse-Z (1 = near, 0 = far).

struct Uniforms {
    mvp: mat4x4<f32>,
    // x, y: target size in physical pixels; z: pixels per point.
    viewport: vec4<f32>,
    // Grid: minor spacing, major spacing, half extent, unused.
    params: vec4<f32>,
    color_a: vec4<f32>,
    color_b: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

fn px_to_ndc(px: vec2<f32>) -> vec2<f32> {
    return px * 2.0 / u.viewport.xy;
}

// ---------------------------------------------------------------- points

struct PointIn {
    @location(0) offset: vec3<f32>,
    @location(1) size_px: f32,
    @location(2) color: vec4<f32>,
    @location(3) shape: u32,
};

struct PointOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) local_px: vec2<f32>,
    @location(2) radius_px: f32,
    @location(3) @interpolate(flat) shape: u32,
};

fn quad_corner(vi: u32) -> vec2<f32> {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    return corners[vi];
}

@vertex
fn vs_point(@builtin(vertex_index) vi: u32, inst: PointIn) -> PointOut {
    let corner = quad_corner(vi);
    let clip = u.mvp * vec4<f32>(inst.offset, 1.0);
    let radius = inst.size_px * u.viewport.z * 0.5;
    let half = radius + 1.0; // one extra pixel for anti-aliasing
    var out: PointOut;
    // Nudge markers slightly towards the camera so they sit on top of lines.
    let z = min(clip.z + 1e-5 * clip.w, clip.w);
    out.pos = vec4<f32>(clip.xy + px_to_ndc(corner * half) * clip.w, z, clip.w);
    out.color = inst.color;
    out.local_px = corner * half;
    out.radius_px = radius;
    out.shape = inst.shape;
    return out;
}

@fragment
fn fs_point(in: PointOut) -> @location(0) vec4<f32> {
    let p = in.local_px;
    let r = in.radius_px;
    var d: f32;
    switch in.shape {
        case 1u: { d = max(abs(p.x), abs(p.y)) - r; }
        case 2u: {
            let arm = max(0.75, r * 0.2);
            d = max(min(abs(p.x), abs(p.y)) - arm, max(abs(p.x), abs(p.y)) - r);
        }
        default: { d = length(p) - r; }
    }
    let coverage = clamp(0.5 - d, 0.0, 1.0);
    if coverage <= 0.0 {
        discard;
    }
    return vec4<f32>(in.color.rgb, in.color.a * coverage);
}

// ---------------------------------------------------------------- lines

struct LineIn {
    @location(0) a: vec3<f32>,
    @location(1) width_px: f32,
    @location(2) b: vec3<f32>,
    @location(3) color: vec4<f32>,
};

struct LineOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) across_px: f32,
    @location(2) half_width_px: f32,
};

@vertex
fn vs_line(@builtin(vertex_index) vi: u32, inst: LineIn) -> LineOut {
    var ends = array<f32, 6>(0.0, 0.0, 1.0, 0.0, 1.0, 1.0);
    var sides = array<f32, 6>(-1.0, 1.0, 1.0, -1.0, 1.0, -1.0);
    let end = ends[vi];
    let side = sides[vi];

    var ca = u.mvp * vec4<f32>(inst.a, 1.0);
    var cb = u.mvp * vec4<f32>(inst.b, 1.0);
    // Clip against the camera plane (perspective) so lines through the eye stay sane.
    let eps = 1e-5;
    if ca.w < eps && cb.w >= eps {
        ca = mix(ca, cb, (eps - ca.w) / (cb.w - ca.w));
    } else if cb.w < eps && ca.w >= eps {
        cb = mix(cb, ca, (eps - cb.w) / (ca.w - cb.w));
    }

    let sa = ca.xy / ca.w * u.viewport.xy * 0.5;
    let sb = cb.xy / cb.w * u.viewport.xy * 0.5;
    var dir = sb - sa;
    let len = length(dir);
    dir = select(vec2<f32>(1.0, 0.0), dir / max(len, 1e-6), len > 1e-6);
    let normal = vec2<f32>(-dir.y, dir.x);

    let half_width = inst.width_px * u.viewport.z * 0.5;
    let half = half_width + 1.0;
    let clip = select(ca, cb, end > 0.5);
    let offset_px = normal * side * half + dir * (end * 2.0 - 1.0) * half_width;

    var out: LineOut;
    out.pos = vec4<f32>(clip.xy + px_to_ndc(offset_px) * clip.w, clip.z, clip.w);
    out.color = inst.color;
    out.across_px = side * half;
    out.half_width_px = half_width;
    if ca.w < eps && cb.w < eps {
        out.pos = vec4<f32>(0.0, 0.0, 0.0, 1.0); // entirely behind the camera
    }
    return out;
}

@fragment
fn fs_line(in: LineOut) -> @location(0) vec4<f32> {
    let coverage = clamp(in.half_width_px + 0.5 - abs(in.across_px), 0.0, 1.0);
    if coverage <= 0.0 {
        discard;
    }
    return vec4<f32>(in.color.rgb, in.color.a * coverage);
}

// ---------------------------------------------------------------- grid

struct GridOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>,
};

@vertex
fn vs_grid(@builtin(vertex_index) vi: u32) -> GridOut {
    let local = quad_corner(vi) * u.params.z;
    var out: GridOut;
    out.pos = u.mvp * vec4<f32>(local, 0.0, 1.0);
    out.local = local;
    return out;
}

fn grid_coverage(coord: vec2<f32>) -> f32 {
    let w = fwidth(coord);
    let g = abs(fract(coord - 0.5) - 0.5) / max(w, vec2<f32>(1e-6));
    return 1.0 - min(min(g.x, g.y), 1.0);
}

@fragment
fn fs_grid(in: GridOut) -> @location(0) vec4<f32> {
    // The grid origin is a multiple of the major spacing, so local coordinates
    // line up with world grid lines exactly.
    let minor = grid_coverage(in.local / u.params.x) * u.color_a.a;
    let major = grid_coverage(in.local / u.params.y) * u.color_b.a;
    let fade = 1.0 - smoothstep(0.5, 1.0, length(in.local) / u.params.z);
    let alpha = max(minor, major) * fade;
    if alpha <= 0.002 {
        discard;
    }
    let rgb = select(u.color_a.rgb, u.color_b.rgb, major >= minor);
    return vec4<f32>(rgb, alpha);
}
