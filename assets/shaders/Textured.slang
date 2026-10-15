// A shader that samples, so the descriptor path is proven rather than compiled.
//
// `Triangle.slang` deliberately does *not* read a texture: at the time it was
// written there was no descriptor path to read one with, and a shader that tried
// would have been testing something that did not exist. Now there is one, and
// this is the shader that closes the loop -- a bind group that creates cleanly
// proves nothing, because a bind group that binds nothing also creates cleanly.
//
// ## The bindings, and why they are two
//
// Set 0, binding 0 is a `SampledImage` and binding 1 is a `Sampler`. Two, not
// one `CombinedImageSampler`, because **wgpu cannot express a combined
// descriptor at all** -- WebGPU has no `CombinedImageSampler`, so a shader using
// one would compile for Vulkan and be unbuildable on the second backend.
//
// Two bindings is the common subset. That is a deliberate constraint on what this
// shader may do, not a simplification: a test shader that uses the convenient
// thing on one backend is a test that cannot run on the other.

// Set 0, binding 0: a sampled image.
//
// `[[vk::binding]]`, **not** `register(t0)`. `register` is the D3D convention;
// slangc warns with `E39029` that a D3D register with no Vulkan binding or
// `-fvk-t-shift` leaves the Vulkan binding unstated, and `E39001` that the
// texture and sampler registers *overlap*. Both are warnings, not errors, so the
// module compiles and the bindings are simply not the ones anyone expected --
// which is the worst possible failure mode for a test whose whole job is to
// check a binding.
//
// Explicit and unreadable-as-a-mistake beats implicit and correct-by-accident.
[[vk::binding(0, 0)]] Texture2D<float4> albedo;
// Set 0, binding 1: a sampler, separate because the backend above requires it.
//
// `SamplerState`, not `Sampler`. Slang follows HLSL here and the name is not
// obvious: `Sampler` is a bare identifier that does not resolve, and the error
// points at the declaration rather than at the concept.
[[vk::binding(1, 0)]] SamplerState albedoSampler;

struct VsInput {
    float3 position : POSITION;
    float2 uv       : TEXCOORD0;
};

struct VsOutput {
    float4 position : SV_Position;
    float2 uv       : TEXCOORD0;
};

// Positions are already in clip space and UVs already run 0..1, so the test
// cannot fail for a reason that is not the backend: there is no camera matrix
// and no transform to get wrong.
[shader("vertex")]
VsOutput vs_main(VsInput input)
{
    VsOutput output;
    output.position = float4(input.position, 1.0f);
    output.uv = input.uv;
    return output;
}

/// Samples the texture and returns it unchanged.
///
/// No tint, no gamma, no tone curve. The point is that the pixel that comes out
/// is the texel that went in, so a test can name the exact colour it expects and
/// a wrong one names the backend rather than a maths mistake.
[shader("fragment")]
float4 fs_main(VsOutput input) : SV_Target0
{
    return albedo.Sample(albedoSampler, input.uv);
}
