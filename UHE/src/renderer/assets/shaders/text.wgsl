// The MSDF text draw. One glyph atlas and one view-projection per draw call: the
// layout puts every glyph of a run in one vertex buffer, so a run is one
// `draw_indexed` rather than the C++ one-per-glyph path with its 32-slot
// descriptor array.
//
// ## Bindings
//
// Set 0, binding 0 is a `SampledImage`, binding 1 a `Sampler` -- separate, for
// the same reason the root `assets/shaders/Textured.slang` documents: wgpu has
// no combined descriptor. Set 1, binding 0 is the view-projection and the atlas
// parameters the C++ spelt as a push constant (`Renderer2D.cpp:29-33`); the RHI
// has no push-constant range, so it is a uniform buffer, and separating it from
// set 0 is what lets a pass bind it once for every text run.
//
// ## Two atlas kinds, one pipeline
//
// `globals.atlas.w` selects the reconstruction. A baked `msdf-atlas-gen` atlas
// is a signed distance field and takes the median-of-three screen-space range
// path the C++ shader used (the root `assets/shaders/Text.slang`), with one correction:
// the C++ divided the pixel range by a hardcoded `float2(1024.0, 1024.0)`, which
// is wrong for every atlas that is not 1024x1024, so the real atlas size and
// distance range come from the atlas instead. The atlas `font::generate` builds
// at startup out of `ab_glyph` is plain coverage, where the alpha *is* the
// coverage and there is no range to widen. `select`, not a branch, so `fwidth`
// stays in uniform control flow -- the WGSL rule that catches a derivative
// behind an `if`.

struct Globals {
    view_projection: mat4x4<f32>,
    // (width, height, distance range, mode: 1 msdf, 0 coverage)
    atlas: vec4<f32>,
};

@group(0) @binding(0) var text_atlas: texture_2d<f32>;
@group(0) @binding(1) var text_sampler: sampler;
@group(1) @binding(0) var<uniform> globals: Globals;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
};

fn median(r: f32, g: f32, b: f32) -> f32 {
    return max(min(r, g), min(max(r, g), b));
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.clip_position = globals.view_projection * vec4<f32>(input.position, 1.0);
    output.color = input.color;
    output.uv = input.uv;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let texel = textureSample(text_atlas, text_sampler, input.uv);

    let unit_range = vec2<f32>(
        globals.atlas.z / globals.atlas.x,
        globals.atlas.z / globals.atlas.y,
    );
    let screen_tex_size = vec2<f32>(1.0, 1.0) / fwidth(input.uv);
    let screen_px_range = max(0.5 * dot(unit_range, screen_tex_size), 1.0);
    let sig_dist = median(texel.r, texel.g, texel.b) - 0.5;
    let msdf_coverage = clamp(sig_dist * screen_px_range + 0.5, 0.0, 1.0);

    let coverage = select(texel.r, msdf_coverage, globals.atlas.w > 0.5);

    let color = vec4<f32>(input.color.rgb, input.color.a * coverage);
    if (color.a < 0.01) {
        discard;
    }
    return color;
}
