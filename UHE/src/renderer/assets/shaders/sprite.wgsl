// The 2D sprite batch draw. One texture per draw call, which is what the batch
// guarantees (see `src/batch2d.rs`), so this samples one texture rather than the
// C++ 32-slot descriptor array.
//
// ## Bindings
//
// Set 0, binding 0 is a `SampledImage` and binding 1 is a `Sampler` -- two
// bindings, not one `CombinedImageSampler`, because wgpu cannot express a
// combined descriptor at all (the root `assets/shaders/Textured.slang` carries
// the same note for the same reason). Set 1, binding 0 is the view-projection
// the C++ spelt as a push constant (`Renderer2D.cpp:29-33`); the RHI has no
// push-constant range, so it is a uniform buffer, and separating it from set 0
// is what lets a pass bind it once for every batch.

struct Globals {
    view_projection: mat4x4<f32>,
};

@group(0) @binding(0) var sprite_texture: texture_2d<f32>;
@group(0) @binding(1) var sprite_sampler: sampler;
@group(1) @binding(0) var<uniform> globals: Globals;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) tiling_factor: f32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) tiling_factor: f32,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.clip_position = globals.view_projection * vec4<f32>(input.position, 1.0);
    output.color = input.color;
    output.uv = input.uv;
    output.tiling_factor = input.tiling_factor;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let texel = textureSample(sprite_texture, sprite_sampler, input.uv * input.tiling_factor);
    let color = texel * input.color;
    if (color.a < 0.01) {
        discard;
    }
    return color;
}
