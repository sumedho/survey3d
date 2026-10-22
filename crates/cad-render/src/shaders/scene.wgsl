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

// ---------------------------------------------------------------- symbols

// Point symbols and line patterns (see `survey_core::style`). Each symbol is a
// run of primitives, each two vec4s: (kind, fill, x, y) and its parameters.
struct SymbolTable {
    // (first primitive, count, -, -) per symbol.
    ranges: array<vec4<u32>, 64>,
    prims: array<vec4<f32>, 256>,
    // On/off lengths in line widths per pattern id (0 = solid).
    patterns: array<vec4<f32>, 4>,
};

@group(0) @binding(1) var<uniform> symbols: SymbolTable;

const PI: f32 = 3.14159265;

fn sd_segment(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-12), 0.0, 1.0);
    return length(pa - ba * h);
}

fn sd_box(p: vec2<f32>, h: f32) -> f32 {
    let d = abs(p) - vec2<f32>(h);
    return length(max(d, vec2<f32>(0.0))) + min(max(d.x, d.y), 0.0);
}

// Corner `i` of a regular polygon (first corner `rot` clockwise from up) or of a
// star (alternating outer and inner radius).
fn poly_vertex(c: vec2<f32>, r: f32, n: f32, param: f32, star: bool, i: u32) -> vec2<f32> {
    var a: f32;
    var radius = r;
    if star {
        a = f32(i) * PI / n;
        if (i % 2u) == 1u {
            radius = r * param;
        }
    } else {
        a = param + f32(i) * 2.0 * PI / n;
    }
    return c + radius * vec2<f32>(sin(a), cos(a));
}

// Signed distance to a polygon or star (negative inside).
fn sd_poly(p: vec2<f32>, c: vec2<f32>, r: f32, n: f32, param: f32, star: bool) -> f32 {
    var count = u32(n);
    if star {
        count = count * 2u;
    }
    var d = 1e9;
    var s = 1.0;
    var j = count - 1u;
    for (var i = 0u; i < count; i = i + 1u) {
        let vi = poly_vertex(c, r, n, param, star, i);
        let vj = poly_vertex(c, r, n, param, star, j);
        let e = vj - vi;
        let w = p - vi;
        let b = w - e * clamp(dot(w, e) / max(dot(e, e), 1e-12), 0.0, 1.0);
        d = min(d, dot(b, b));
        let c1 = p.y >= vi.y;
        let c2 = p.y < vj.y;
        let c3 = e.x * w.y > e.y * w.x;
        if (c1 && c2 && c3) || (!c1 && !c2 && !c3) {
            s = -s;
        }
        j = i;
    }
    return s * sqrt(d);
}

// Distance (in unit symbol space) to symbol `id`, strokes `half` thick.
fn sd_symbol(p: vec2<f32>, id: u32, half: f32) -> f32 {
    let range = symbols.ranges[min(id, 63u)];
    var d = 1e9;
    for (var k = 0u; k < range.y; k = k + 1u) {
        let i = (range.x + k) * 2u;
        let a = symbols.prims[i];
        let b = symbols.prims[i + 1u];
        let kind = u32(a.x + 0.5);
        let fill = a.y > 0.5;
        let c = a.zw;
        var dk: f32;
        switch kind {
            case 0u: { dk = length(p - c) - b.x; }
            case 1u: { dk = sd_box(p - c, b.x); }
            case 2u: { dk = sd_poly(p, c, b.x, b.y, b.z, false); }
            case 3u: { dk = sd_poly(p, c, b.x, b.y, b.z, true); }
            default: { dk = sd_segment(p, c, b.xy) - half; }
        }
        if kind < 4u && !fill {
            dk = abs(dk) - half;
        }
        d = min(d, dk);
    }
    return d;
}

// ---------------------------------------------------------------- points

struct PointIn {
    @location(0) offset: vec3<f32>,
    @location(1) size_px: f32,
    @location(2) color: vec4<f32>,
    @location(3) symbol: u32,
};

struct PointOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) local_px: vec2<f32>,
    @location(2) radius_px: f32,
    @location(3) @interpolate(flat) symbol: u32,
};

fn quad_corner(vi: u32) -> vec2<f32> {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    return corners[vi];
}

fn stroke_px(radius: f32) -> f32 {
    return max(1.1, radius * 0.17);
}

@vertex
fn vs_point(@builtin(vertex_index) vi: u32, inst: PointIn) -> PointOut {
    let corner = quad_corner(vi);
    let clip = u.mvp * vec4<f32>(inst.offset, 1.0);
    let radius = inst.size_px * u.viewport.z * 0.5;
    // Room for strokes on the unit circle plus a pixel for anti-aliasing.
    let half = radius + stroke_px(radius) * 0.5 + 1.0;
    var out: PointOut;
    // Nudge markers slightly towards the camera so they sit on top of lines.
    let z = min(clip.z + 1e-5 * clip.w, clip.w);
    out.pos = vec4<f32>(clip.xy + px_to_ndc(corner * half) * clip.w, z, clip.w);
    out.color = inst.color;
    out.local_px = corner * half;
    out.radius_px = radius;
    out.symbol = inst.symbol;
    return out;
}

@fragment
fn fs_point(in: PointOut) -> @location(0) vec4<f32> {
    let r = max(in.radius_px, 0.5);
    let half = stroke_px(r) * 0.5 / r;
    let d = sd_symbol(in.local_px / r, in.symbol, half) * r;
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
    @location(4) start: f32,
    @location(5) pattern: u32,
};

struct LineOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) across_px: f32,
    @location(2) half_width_px: f32,
    // Screen distance along the line from its start, for dash patterns.
    @location(3) along_px: f32,
    @location(4) @interpolate(flat) pattern: u32,
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

    // Pixels per world unit along this segment (exact in plan, approximate in perspective).
    let world = length(inst.b - inst.a);
    let scale = select(0.0, len / world, world > 1e-9);

    var out: LineOut;
    out.pos = vec4<f32>(clip.xy + px_to_ndc(offset_px) * clip.w, clip.z, clip.w);
    out.color = inst.color;
    out.across_px = side * half;
    out.half_width_px = half_width;
    out.along_px = inst.start * scale + end * len + (end * 2.0 - 1.0) * half_width;
    out.pattern = inst.pattern;
    if ca.w < eps && cb.w < eps {
        out.pos = vec4<f32>(0.0, 0.0, 0.0, 1.0); // entirely behind the camera
    }
    return out;
}

// Whether `along` (in line widths) falls on a dash of pattern `id`.
fn pattern_on(along: f32, id: u32) -> bool {
    let pat = symbols.patterns[min(id, 3u)];
    let total = pat.x + pat.y + pat.z + pat.w;
    if id == 0u || total <= 0.0 {
        return true;
    }
    let t = along - floor(along / total) * total;
    return t < pat.x || (t >= pat.x + pat.y && t < pat.x + pat.y + pat.z);
}

@fragment
fn fs_line(in: LineOut) -> @location(0) vec4<f32> {
    let coverage = clamp(in.half_width_px + 0.5 - abs(in.across_px), 0.0, 1.0);
    let unit = max(in.half_width_px * 2.0, 1.5 * u.viewport.z);
    if coverage <= 0.0 || !pattern_on(in.along_px / unit, in.pattern) {
        discard;
    }
    return vec4<f32>(in.color.rgb, in.color.a * coverage);
}

// ---------------------------------------------------------------- triangles

struct TriIn {
    @location(0) offset: vec3<f32>,
    @location(1) color: vec4<f32>,
};

struct TriOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_tri(inst: TriIn) -> TriOut {
    let clip = u.mvp * vec4<f32>(inst.offset, 1.0);
    var out: TriOut;
    // Keep lines and point markers lying on a surface visible: in plan view
    // surfaces are squeezed into the farthest sliver of depth (keeping their
    // order, so overlapping surfaces still sort by height); in 3D they are
    // pushed a fraction of their depth away (reverse-Z: smaller is farther),
    // so terrain still hides what is behind it.
    var z = clip.z * (1.0 - 3e-3);
    if u.params.w > 0.5 {
        z = clip.z * 1e-3;
    }
    out.pos = vec4<f32>(clip.xy, z, clip.w);
    out.color = inst.color;
    return out;
}

@fragment
fn fs_tri(in: TriOut) -> @location(0) vec4<f32> {
    return in.color;
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
