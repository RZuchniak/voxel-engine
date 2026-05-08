struct CameraUniform {
    view_proj: mat4x4<f32>,
};
@group(0) @binding(0)
var<uniform> camera: CameraUniform;
@group(1) @binding(0)
var block_texture: texture_2d_array<f32>;
@group(1) @binding(1)
var block_sampler: sampler;

struct VertexInput {
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) tex_layer: u32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) tex_layer: u32,
    @location(2) ndc_depth: f32,
};

@vertex
fn vs_main(
model: VertexInput,
) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = camera.view_proj * vec4<f32>(model.pos, 1.0);
    out.uv = model.uv;
    out.tex_layer = model.tex_layer;
    out.ndc_depth = out.clip_position.z / out.clip_position.w;
    return out;
}

@fragment
fn fs_main(
    @location(0) uv: vec2<f32>,
    @location(1) tex_layer: u32,
    @location(2) ndc_depth: f32,
) -> @location(0) vec4<f32> {
    let albedo = textureSample(block_texture, block_sampler, uv, i32(tex_layer));
    let sky = vec3<f32>(0.49, 0.74, 0.95);

    // Simple atmospheric fog to smooth far-distance transitions.
    let fog_start = 0.55;
    let fog_end = 0.98;
    let fog = clamp((ndc_depth - fog_start) / (fog_end - fog_start), 0.0, 1.0);
    let rgb = mix(albedo.rgb, sky, fog);
    return vec4<f32>(rgb, albedo.a);
}
