// Gizmo lines: a line list with a per-vertex colour.
//
// Not a port: the C++ draws nothing but the grid, so an editor with anything
// else to draw had to roll its own pass. Two matrices — the camera's and the
// batch's own model matrix — and one vertex buffer.

struct LineFrame {
    view_projection: mat4x4<f32>,
    model: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> frame: LineFrame;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip = frame.view_projection * frame.model * vec4<f32>(in.position, 1.0);
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
