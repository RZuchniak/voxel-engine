//! Are there holes in solid geometry where two faces meet?
//!
//! Renders a solid 16³ block of stone headlessly, angled so the convex edge between its +X face
//! and its top face crosses the frame, then looks for **background pixels with covered pixels
//! both above and below them in the same column** (and the same by row). A convex solid cannot
//! have a hole in its silhouette, so every one of those is a seam you can see through.
//!
//! This is the pixel-level version of `correctness_meshing::opaque_faces_land_on_the_block_lattice`.
//! It stays a diagnostic rather than a test because the seam is *sub-pixel* — 0.002 blocks wide —
//! so whether it lands on a pixel centre depends on the exact camera, and a test that can pass by
//! luck is worse than no test. The lattice check is the deterministic guard.
//!
//! Run: `cargo run --release --example diag_face_seams`

use voxel_engine::{
    block::BlockId,
    mesh::mesh_chunk,
    world::{Chunk, World, SECTION_SIZE},
    Vertex,
};

const TARGET: u32 = 512;

const SHADER: &str = r#"
struct Uniforms {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: Uniforms;

@vertex
fn vs_main(@location(0) pos: vec3<f32>) -> @builtin(position) vec4<f32> {
    return u.view_proj * vec4<f32>(pos - u.camera_pos.xyz, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return vec4<f32>(1.0, 1.0, 1.0, 1.0);
}
"#;

fn solid_chunk(coord: (i32, i32)) -> Chunk {
    let mut chunk = Chunk::new(coord);
    for z in 0..SECTION_SIZE {
        for x in 0..SECTION_SIZE {
            for y in 0..SECTION_SIZE as i32 {
                chunk.set_block_world(x, y, z, BlockId::STONE);
            }
        }
    }
    chunk
}

fn view_proj(eye: [f32; 3], target: [f32; 3]) -> cgmath::Matrix4<f32> {
    use cgmath::{EuclideanSpace, Point3, Vector3};
    let eye = Point3::new(eye[0], eye[1], eye[2]);
    let target = Point3::new(target[0], target[1], target[2]);
    let view = cgmath::Matrix4::look_at_rh(
        Point3::origin(),
        Point3::from_vec(target - eye),
        Vector3::unit_y(),
    );
    let proj = cgmath::perspective(cgmath::Deg(70.0), 1.0, 0.1, 500.0);
    proj * view
}

fn wgpu_buffer(device: &wgpu::Device, contents: &[u8], usage: wgpu::BufferUsages) -> wgpu::Buffer {
    use wgpu::util::DeviceExt;
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents,
        usage,
    })
}

/// One bit per pixel: was it covered by geometry?
fn render_coverage(eye: [f32; 3], target: [f32; 3]) -> Option<Vec<bool>> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        compatible_surface: None,
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
    }))
    .ok()?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: wgpu::Features::empty(),
        ..Default::default()
    }))
    .ok()?;

    let mut world = World::new();
    world.insert_chunk(solid_chunk((0, 0)));
    let meshes = mesh_chunk(&world, (0, 0));
    let mut vertices: Vec<Vertex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    for (_, mesh) in &meshes {
        let base = vertices.len() as u32;
        vertices.extend_from_slice(mesh.vertices());
        indices.extend(mesh.indices().iter().map(|i| i + base));
    }

    let mut uniform = [0f32; 20];
    uniform[..16].copy_from_slice(AsRef::<[f32; 16]>::as_ref(&view_proj(eye, target)));
    uniform[16..19].copy_from_slice(&eye);

    let uniform_buffer =
        wgpu_buffer(&device, bytemuck::cast_slice(&uniform), wgpu::BufferUsages::UNIFORM);
    let vertex_buffer =
        wgpu_buffer(&device, bytemuck::cast_slice(&vertices), wgpu::BufferUsages::VERTEX);
    let index_buffer =
        wgpu_buffer(&device, bytemuck::cast_slice(&indices), wgpu::BufferUsages::INDEX);

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });
    let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &bind_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform_buffer.as_entire_binding(),
        }],
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[&bind_layout],
        push_constant_ranges: &[],
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Vertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &[wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x3,
                }],
            }],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba8Unorm,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            polygon_mode: wgpu::PolygonMode::Fill,
            cull_mode: Some(wgpu::Face::Back),
            ..Default::default()
        },
        depth_stencil: None,
        multisample: Default::default(),
        multiview: None,
        cache: None,
    });

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: TARGET,
            height: TARGET,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (TARGET * TARGET * 4) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..indices.len() as u32, 0, 0..1);
    }
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(TARGET * 4),
                rows_per_image: Some(TARGET),
            },
        },
        wgpu::Extent3d {
            width: TARGET,
            height: TARGET,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));

    readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::Wait);
    let data = readback.slice(..).get_mapped_range();
    let coverage: Vec<bool> = data.chunks_exact(4).map(|px| px[0] > 128).collect();
    drop(data);
    readback.unmap();
    Some(coverage)
}

/// Uncovered pixels that have covered pixels on both sides along one axis. The rendered object
/// is convex, so its silhouette is convex too: any such pixel is a hole in the surface.
fn holes(coverage: &[bool], by_column: bool) -> usize {
    let n = TARGET as usize;
    let at = |x: usize, y: usize| coverage[y * n + x];
    let mut found = 0;
    for outer in 0..n {
        let line: Vec<bool> = (0..n)
            .map(|inner| {
                if by_column {
                    at(outer, inner)
                } else {
                    at(inner, outer)
                }
            })
            .collect();
        let Some(first) = line.iter().position(|c| *c) else {
            continue;
        };
        let last = line.iter().rposition(|c| *c).unwrap();
        found += line[first..=last].iter().filter(|c| !**c).count();
    }
    found
}

fn main() {
    // Angled at the convex edge where the +X face meets the top face, so it crosses the frame
    // diagonally and a long run of pixel centres can fall into any gap.
    let views: [([f32; 3], [f32; 3], &str); 3] = [
        ([26.0, 26.0, 8.0], [16.0, 16.0, 8.0], "+X/top edge, head on"),
        ([30.0, 24.0, 30.0], [8.0, 16.0, 8.0], "top corner, three faces"),
        ([24.0, 20.0, 22.0], [12.0, 14.0, 12.0], "shallow angle across two edges"),
    ];

    let mut total = 0;
    for (eye, target, label) in views {
        let Some(coverage) = render_coverage(eye, target) else {
            eprintln!("no GPU adapter available; cannot measure");
            return;
        };
        let covered = coverage.iter().filter(|c| **c).count();
        let col = holes(&coverage, true);
        let row = holes(&coverage, false);
        total += col + row;
        println!(
            "{label:<34} covered {covered:>6} px   holes: {col} by column, {row} by row"
        );
    }
    println!(
        "\n{}",
        if total == 0 {
            "no see-through seams".to_string()
        } else {
            format!("{total} see-through pixels inside the silhouette")
        }
    );
}
