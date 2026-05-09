struct CameraUniform {
    view_proj: mat4x4<f32>,
};
struct SkyUniform {
    top_color: vec4<f32>,
    horizon_color: vec4<f32>,
    fog_params: vec4<f32>,
    camera_pos: vec4<f32>,
};
@group(0) @binding(0)
var<uniform> camera: CameraUniform;
@group(0) @binding(1)
var<uniform> sky: SkyUniform;
@group(1) @binding(0)
var block_texture: texture_2d_array<f32>;
@group(1) @binding(1)
var block_sampler: sampler;

struct VertexInput {
    @location(0) pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) tex_layer: u32,
    @location(3) light: u32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) tex_layer: u32,
    @location(2) world_pos: vec3<f32>,
    @location(3) light: f32,
};

@vertex
fn vs_main(
model: VertexInput,
) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = camera.view_proj * vec4<f32>(model.pos, 1.0);
    out.uv = model.uv;
    out.tex_layer = model.tex_layer;
    out.world_pos = model.pos;
    out.light = f32(model.light) / 255.0;
    return out;
}

@fragment
fn fs_main(
    @location(0) uv: vec2<f32>,
    @location(1) tex_layer: u32,
    @location(2) world_pos: vec3<f32>,
    @location(3) light: f32,
) -> @location(0) vec4<f32> {
    let albedo = textureSample(block_texture, block_sampler, uv, i32(tex_layer));
    let height_lerp = clamp(world_pos.y / 256.0, 0.0, 1.0);
    let sky_color = mix(sky.horizon_color.rgb, sky.top_color.rgb, height_lerp);
    let lit_albedo = albedo.rgb * (0.25 + light * 0.75);

    // Distance fog: smoothstep so far geometry blends into the sky/horizon.
    let fog_start = sky.fog_params.x;
    let fog_end = sky.fog_params.y;
    let fog_strength = sky.fog_params.z;
    let dist = distance(world_pos, sky.camera_pos.xyz);
    let fog = smoothstep(fog_start, fog_end, dist) * fog_strength;
    let rgb = mix(lit_albedo, sky_color, fog);
    return vec4<f32>(rgb, albedo.a);
}
