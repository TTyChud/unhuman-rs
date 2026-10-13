// The smallest shader that proves a draw actually reaches the framebuffer.
//
// Two entry points in one module, compiled to two SPIR-V binaries, because
// Vulkan has no linked shader stage: `slangc -target spirv` emits one module per
// entry point and the backend loads them as a pair.

struct VsInput {
    float3 position : POSITION;
    float4 color    : COLOR;
};

struct VsOutput {
    float4 position : SV_Position;
    float4 color    : COLOR;
};

// Positions are already in clip space: no camera matrix, so the test cannot fail
// for a reason that is not the backend.
[shader("vertex")]
VsOutput vs_main(VsInput input)
{
    VsOutput output;
    output.position = float4(input.position, 1.0f);
    output.color = input.color;
    return output;
}

// A constant colour, not a material. Anything that reads a texture here would be
// testing the descriptor path before there is a descriptor path to test.
[shader("fragment")]
float4 fs_main(VsOutput input) : SV_Target0
{
    return input.color;
}
