// Georeferenced image tiles: textured quads under the drawing.

struct ImageUniforms {
    mvp: mat4x4<f32>,
    // x: opacity.
    params: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: ImageUniforms;
@group(1) @binding(0) var tile: texture_2d<f32>;
@group(1) @binding(1) var tile_sampler: sampler;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_image(@location(0) position: vec3<f32>, @location(1) uv: vec2<f32>) -> VertexOut {
    var out: VertexOut;
    out.position = u.mvp * vec4<f32>(position, 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn fs_image(in: VertexOut) -> @location(0) vec4<f32> {
    let c = textureSample(tile, tile_sampler, in.uv);
    return vec4<f32>(c.rgb, c.a * u.params.x);
}
