// The editor grid. Port of `UHE_EDITOR/assets/shaders/Grid.slang`.
//
// Six vertices from `@builtin(vertex_index)` cover the viewport; the shader
// unprojects the near and far planes, intersects that ray with the y = 0 plane,
// and draws a fading grid with a screen-space-constant line width. The axis
// lines are coloured: x red, z blue.
//
// Two differences from the Slang original, both forced:
//   * the two matrices arrived as push constants, which this RHI cannot declare,
//     so they are a uniform block;
//   * `SV_Depth` has no WGSL spelling on the output (`@builtin(frag_depth)` does,
//     and is what is used), so the fragment still writes the real 3D depth and
//     the grid is occluded by geometry correctly.

struct Grid {
    view_projection: mat4x4<f32>,
    inverse_view_projection: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> grid: Grid;

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) near_point: vec3<f32>,
    @location(1) far_point: vec3<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    var positions = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, 1.0),
    );

    let pos = positions[index];

    // Depth 0 is the near plane and 1 the far one, because the projection is
    // 0..1 (`GLM_FORCE_DEPTH_ZERO_TO_ONE`).
    let near_clip = vec4<f32>(pos.x, pos.y, 0.0, 1.0);
    let far_clip = vec4<f32>(pos.x, pos.y, 1.0, 1.0);

    let unprojected_near = grid.inverse_view_projection * near_clip;
    let unprojected_far = grid.inverse_view_projection * far_clip;

    var out: VertexOutput;
    out.near_point = unprojected_near.xyz / unprojected_near.w;
    out.far_point = unprojected_far.xyz / unprojected_far.w;
    out.clip = vec4<f32>(pos, 0.9999, 1.0);
    return out;
}

struct FragmentOutput {
    @location(0) color: vec4<f32>,
    // Grid.slang's `SV_Depth`.
    @builtin(frag_depth) depth: f32,
};

@fragment
fn fs_main(in: VertexOutput) -> FragmentOutput {
    var out: FragmentOutput;

    // Where the ray crosses y = 0.
    let t = -in.near_point.y / (in.far_point.y - in.near_point.y);
    if (t < 0.0) {
        discard;
    }

    let frag_pos = in.near_point + t * (in.far_point - in.near_point);

    let coord = frag_pos.xz;
    let derivative = fwidth(coord);
    let cell = abs(fract(coord - 0.5) - 0.5) / derivative;
    let line = min(cell.x, cell.y);
    let minimum_z = min(derivative.y, 1.0);
    let minimum_x = min(derivative.x, 1.0);

    var color = vec4<f32>(0.4, 0.4, 0.4, 1.0 - min(line, 1.0));

    // The primary axes: z blue, x red.
    if (frag_pos.x > -0.5 * minimum_x && frag_pos.x < 0.5 * minimum_x) {
        color = vec4<f32>(0.1, 0.1, 0.8, color.a);
    }
    if (frag_pos.z > -0.5 * minimum_z && frag_pos.z < 0.5 * minimum_z) {
        color = vec4<f32>(0.8, 0.1, 0.1, color.a);
    }

    // Fade with distance, cubically so the horizon does not alias into noise.
    let linear_depth = length(frag_pos - in.near_point);
    var fade = max(0.0, 1.0 - linear_depth / 50.0);
    fade = fade * fade * fade;
    color.a = color.a * fade * 0.5;

    if (color.a < 0.01) {
        discard;
    }

    let clip = grid.view_projection * vec4<f32>(frag_pos, 1.0);
    out.color = color;
    out.depth = clip.z / clip.w;
    return out;
}
