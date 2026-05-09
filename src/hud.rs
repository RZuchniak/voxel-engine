use std::borrow::Cow;

use font8x8::legacy::BASIC_LEGACY;
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct HudVertex {
    position: [f32; 2],
    uv: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ScreenUniform {
    size: [f32; 2],
}

const PAD_PX: f32 = 10.0;
const SCALE: usize = 2;
const LINE_GAP: usize = 2;
const CHAR_GAP: usize = 1;
const MAX_TEX_W: u32 = 720;
const MAX_TEX_H: u32 = 220;

pub struct HudOverlay {
    texture: wgpu::Texture,
    // Kept for GPU resource lifetime (bind group references).
    #[allow(dead_code)]
    texture_view: wgpu::TextureView,
    #[allow(dead_code)]
    sampler: wgpu::Sampler,
    screen_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    raster_buf: Vec<u8>,
}

impl HudOverlay {
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("HUD texture"),
            size: wgpu::Extent3d {
                width: MAX_TEX_W,
                height: MAX_TEX_H,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("HUD sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let screen_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("HUD screen uniform"),
            contents: bytemuck::bytes_of(&ScreenUniform {
                size: [1280.0, 720.0],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("HUD bind group layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("HUD bind group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: screen_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("HUD shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("hud.wgsl"))),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("HUD pipeline layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("HUD pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<HudVertex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 0,
                            format: wgpu::VertexFormat::Float32x2,
                        },
                        wgpu::VertexAttribute {
                            offset: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
                            shader_location: 1,
                            format: wgpu::VertexFormat::Float32x2,
                        },
                    ],
                }],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("HUD vertex buffer"),
            size: (std::mem::size_of::<HudVertex>() * 4) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("HUD index buffer"),
            contents: bytemuck::cast_slice(&[0u32, 1, 2, 0, 2, 3]),
            usage: wgpu::BufferUsages::INDEX,
        });

        Self {
            texture,
            texture_view,
            sampler,
            screen_buffer,
            bind_group,
            pipeline,
            vertex_buffer,
            index_buffer,
            raster_buf: Vec::new(),
        }
    }

    pub fn resize(&mut self, queue: &wgpu::Queue, width: u32, height: u32) {
        let u = ScreenUniform {
            size: [width.max(1) as f32, height.max(1) as f32],
        };
        queue.write_buffer(&self.screen_buffer, 0, bytemuck::bytes_of(&u));
    }

    /// Rasterize `text` (ASCII / newline) into the HUD texture and update the screen-space quad.
    pub fn prepare(&mut self, queue: &wgpu::Queue, text: &str, screen_w: u32, screen_h: u32) {
        self.resize(queue, screen_w, screen_h);

        let cell = 8 * SCALE;
        let char_step = cell + CHAR_GAP;
        let line_step = cell + LINE_GAP;

        let lines: Vec<&str> = text.lines().collect();
        let max_chars = lines.iter().map(|l| l.len()).max().unwrap_or(0);
        let content_w = max_chars * char_step;
        let content_h = lines.len() * line_step;
        let pad = PAD_PX as usize;
        let tex_w = (content_w + pad * 2).min(MAX_TEX_W as usize) as u32;
        let tex_h = (content_h + pad * 2).min(MAX_TEX_H as usize) as u32;

        let npix = (tex_w * tex_h * 4) as usize;
        self.raster_buf.clear();
        self.raster_buf.resize(npix, 0);

        let bw = tex_w as usize;
        let bh = tex_h as usize;
        for (line_i, line) in lines.iter().enumerate() {
            let base_y = pad + line_i * line_step;
            let base_x = pad;
            for (col, byte) in line.bytes().enumerate() {
                let idx = byte as usize;
                let glyph = if idx < 128 {
                    BASIC_LEGACY[idx]
                } else {
                    BASIC_LEGACY[b'?' as usize]
                };
                let px0 = base_x + col * char_step;
                let py0 = base_y;
                blit_glyph(&mut self.raster_buf, bw, bh, px0, py0, &glyph, SCALE);
            }
        }

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &self.raster_buf,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * tex_w),
                rows_per_image: Some(tex_h),
            },
            wgpu::Extent3d {
                width: tex_w,
                height: tex_h,
                depth_or_array_layers: 1,
            },
        );

        let x0 = PAD_PX;
        let y0 = PAD_PX;
        let x1 = x0 + tex_w as f32;
        let y1 = y0 + tex_h as f32;
        let u0 = 0.0f32;
        let v0 = 0.0f32;
        let u1 = tex_w as f32 / MAX_TEX_W as f32;
        let v1 = tex_h as f32 / MAX_TEX_H as f32;

        let verts = [
            HudVertex {
                position: [x0, y0],
                uv: [u0, v0],
            },
            HudVertex {
                position: [x1, y0],
                uv: [u1, v0],
            },
            HudVertex {
                position: [x1, y1],
                uv: [u1, v1],
            },
            HudVertex {
                position: [x0, y1],
                uv: [u0, v1],
            },
        ];
        queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&verts));

    }

    pub fn render<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..6, 0, 0..1);
    }
}

fn blit_glyph(
    buf: &mut [u8],
    buf_w: usize,
    buf_h: usize,
    px: usize,
    py: usize,
    glyph: &[u8; 8],
    scale: usize,
) {
    for row in 0..8 {
        let bits = glyph[row];
        for col in 0..8 {
            if bits & (1 << col) == 0 {
                continue;
            }
            for sy in 0..scale {
                for sx in 0..scale {
                    let x = px + col * scale + sx;
                    let y = py + row * scale + sy;
                    if x >= buf_w || y >= buf_h {
                        continue;
                    }
                    let i = (y * buf_w + x) * 4;
                    if i + 3 < buf.len() {
                        buf[i] = 240;
                        buf[i + 1] = 248;
                        buf[i + 2] = 255;
                        buf[i + 3] = 235;
                    }
                }
            }
        }
    }
}
