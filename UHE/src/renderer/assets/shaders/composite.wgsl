// Composition: the scene colour times the ambient occlusion, into the swapchain
// image.
//
// The C++ draws straight into the swapchain and has no such pass. Both textures
// are the same size as the target, so this is a straight fetch at the pixel's
// own uv with no filtering to speak of, and the triangle covers the viewport.

@group(0) @binding(0) var scene_color: texture_2d<f32>;
@group(0) @binding(1) var ambient_occlusion: texture_2d<f32>;
@group(0) @binding(2) var composite_sampler: sampler;

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    // (-1,-1), (3,-1), (-1,3): one triangle over the viewport.
    let x = f32((index << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(index & 2u) * 2.0 - 1.0;

    var out: VertexOutput;
    out.uv = vec2<f32>(x * 0.5 + 0.5, 0.5 - y * 0.5);
    out.clip = vec4<f32>(x, y, 0.0, 1.0);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSampleLevel(scene_color, composite_sampler, in.uv, 0.0).rgb;
    let ao = textureSampleLevel(ambient_occlusion, composite_sampler, in.uv, 0.0).r;
    return vec4<f32>(color * ao, 1.0);
}
