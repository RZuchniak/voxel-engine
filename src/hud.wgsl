struct ScreenSize {
    size: vec2<f32>,
};

@group(0) @binding(0)
var<uniform> screen: ScreenSize;

@group(0) @binding(1)
var hud_tex: texture_2d<f32>;
@group(0) @binding(2)
var hud_sampler: sampler;

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@location(0) pos: vec2<f32>, @location(1) uv: vec2<f32>) -> VsOut {
    var o: VsOut;
    let ndc_x = pos.x / screen.size.x * 2.0 - 1.0;
    let ndc_y = 1.0 - pos.y / screen.size.y * 2.0;
    o.clip_pos = vec4<f32>(ndc_x, ndc_y, 0.0, 1.0);
    o.uv = uv;
    return o;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(hud_tex, hud_sampler, in.uv);
}
