@vertex
fn vs_main(@location(0) pos: vec2<f32>, @location(1) color: vec3<f32>) -> @builtin(position) vec4<f32> {
	return vec4<f32>(pos, 0.0, 1.0);
}

struct FSOutput {