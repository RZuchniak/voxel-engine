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
    @location(1) @interpolate(flat) tex_layer: u32,
    @location(2) world_pos: vec3<f32>,
    @location(3) light: f32,
    // Constant across the quad, so flat — interpolating it would bleed one face's
    // brightness into its neighbour along the shared edge.
    @location(4) @interpolate(flat) face_shade: f32,
};

/// Vanilla's fixed per-face brightness, indexed as `Direction::shade_index` in `mesh.rs`:
/// up, down, north/south, east/west. This is what makes a cube read as a cube — without a
/// directional term every face of a block samples the same texel value and comes out identical.
/// Written as branches rather than a `const` array because dynamically indexing a module-scope
/// array is not portable across every WGSL backend this ships on.
fn face_shade_factor(index: u32) -> f32 {
    if (index == 0u) { return 1.0; }
    if (index == 1u) { return 0.5; }
    if (index == 2u) { return 0.8; }
    return 0.6;
}

@vertex
fn vs_main(
model: VertexInput,
) -> VertexOutput {
    var out: VertexOutput;
    out.world_pos = model.pos;
    let relative = model.pos - sky.camera_pos.xyz;
    out.clip_position = camera.view_proj * vec4<f32>(relative, 1.0);
    out.uv = model.uv;
    out.tex_layer = model.tex_layer;
    // Low byte is per-vertex occlusion, bits 8..9 select the face brightness. Must match
    // `mesh::FACE_SHADE_SHIFT` / `mesh::AO_MASK`.
    out.light = f32(model.light & 0xFFu) / 255.0;
    out.face_shade = face_shade_factor((model.light >> 8u) & 3u);
    return out;
}

/// Horizontal distance — stable fog when moving vertically (matches section culling).
fn fog_distance(world_pos: vec3<f32>) -> f32 {
    let delta = world_pos - sky.camera_pos.xyz;
    return length(delta.xz);
}

@fragment
fn fs_main(
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) tex_layer: u32,
    @location(2) world_pos: vec3<f32>,
    @location(3) light: f32,
    @location(4) @interpolate(flat) face_shade: f32,
) -> @location(0) vec4<f32> {
    let fog_start = sky.fog_params.x;
    let fog_end = sky.fog_params.y;
    let fog_strength = sky.fog_params.z;
    let dist = fog_distance(world_pos);

    // Extra mip bias at range softens pixel crawl on distant minified texels.
    let fog_t = smoothstep(fog_start, fog_end, dist);
    let mip_bias = fog_t * 1.0;
    let albedo = textureSampleBias(block_texture, block_sampler, uv, i32(tex_layer), mip_bias);
    if (albedo.a < 0.01) {
        discard;
    }

    let height_lerp = clamp(world_pos.y / 256.0, 0.0, 1.0);
    let sky_color = mix(sky.horizon_color.rgb, sky.top_color.rgb, height_lerp);

    // Flatten harsh per-vertex AO contrast in the far field (reduces sparkle). The face term is
    // deliberately *not* flattened with it: it is what conveys shape, and vanilla keeps it at all
    // distances.
    let light_far = mix(light, 0.82, fog_t * 0.65);
    let lit_albedo = albedo.rgb * (0.25 + light_far * 0.75) * face_shade;

    let fog = pow(fog_t, 1.15) * fog_strength;
    let rgb = mix(lit_albedo, sky_color, fog);
    return vec4<f32>(rgb, albedo.a);
}
