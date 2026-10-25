// Composites the point-cloud pass into the scene: fills small holes between
// points, then applies eye-dome lighting (Boucheny 2009, as Potree does it):
// each pixel darkens by how much nearer it is than its neighbours (in log
// depth), outlining shapes without normals. It writes the cloud's depth too,
// so the drawing's lines and surfaces still hide and are hidden by the points.

@group(0) @binding(0) var cloud_color: texture_2d<f32>;
// Linear view depth (0 where there is no point).
@group(0) @binding(1) var cloud_depth: texture_2d<f32>;
@group(0) @binding(2) var cloud_z: texture_depth_2d;
// x: eye-dome lighting strength (0 = off); y: its neighbour radius; z: the
// widest hole filled (all physical px).
@group(0) @binding(3) var<uniform> edl: vec4<f32>;

@vertex
fn vs_edl(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let x = f32((vi << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(vi & 2u) * 2.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

struct Out {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

@fragment
fn fs_edl(@builtin(position) pos: vec4<f32>) -> Out {
    let size = vec2<i32>(textureDimensions(cloud_depth));
    var p = vec2<i32>(pos.xy);
    var d = textureLoad(cloud_depth, p, 0).r;
    if d <= 0.0 {
        // A hole: take the nearest point found along 8 directions, but only
        // when points surround it (6 directions or more), so the cloud's
        // outline and real gaps are not grown.
        let reach = i32(edl.z);
        var found = 0;
        var nearest = 0.0;
        var source = p;
        for (var i = 0; i < 8; i++) {
            let a = f32(i) * 0.785398;
            let dir = vec2<i32>(round(vec2<f32>(cos(a), sin(a))));
            for (var step = 1; step <= reach; step++) {
                let q = p + dir * step;
                if any(q < vec2<i32>(0)) || any(q >= size) {
                    break;
                }
                let n = textureLoad(cloud_depth, q, 0).r;
                if n > 0.0 {
                    found += 1;
                    if nearest == 0.0 || n < nearest {
                        nearest = n;
                        source = q;
                    }
                    break;
                }
            }
        }
        if found < 6 {
            discard;
        }
        p = source;
        d = nearest;
    }
    let color = textureLoad(cloud_color, p, 0);
    var shade = 1.0;
    if edl.x > 0.0 {
        let z = log2(d);
        var sum = 0.0;
        for (var i = 0; i < 8; i++) {
            let a = f32(i) * 0.785398;
            let o = vec2<i32>(round(vec2<f32>(cos(a), sin(a)) * edl.y));
            let q = clamp(p + o, vec2<i32>(0), size - vec2<i32>(1));
            let n = textureLoad(cloud_depth, q, 0).r;
            if n > 0.0 {
                sum += max(0.0, z - log2(n));
            }
        }
        shade = exp(-sum / 8.0 * 300.0 * edl.x);
    }
    var out: Out;
    out.color = vec4<f32>(color.rgb * shade, 1.0);
    out.depth = textureLoad(cloud_z, p, 0);
    return out;
}
