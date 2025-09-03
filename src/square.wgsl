struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec3<f32>, // Output color at location 0
};

@vertex
fn vs_main(
    @location(0) pos: vec2<f32>,
    @location(1) color: vec3<f32> // Input color at location 1
) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = vec4<f32>(pos, 0.0, 1.0);
    out.color = color; // Pass the input color to the output
    return out;
}

@fragment
fn fs_main(@location(0) color: vec3<f32>) -> @location(0) vec4<f32> {
    return vec4<f32>(color, 1.0);
}
