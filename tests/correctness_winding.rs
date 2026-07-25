//! Does the mesher's triangle winding agree with `cull_mode: Some(Face::Back)`?
//!
//! Backface culling is a one-line pipeline change whose correctness is purely visual: get the
//! winding backwards and the world renders inside-out, which no CPU-side assertion would
//! notice. So this renders an actual mesher-produced quad headlessly, through the same
//! transform `square.wgsl` uses (`view_proj * (world_pos - camera_pos)`, camera-relative), and
//! looks at the pixels.
//!
//! The two directions together pin the convention down: a face must be drawn when viewed from
//! its outside and culled when viewed from its inside. Asserting only the first would still
//! pass with culling disabled entirely.

use voxel_engine::{
    block::BlockId,
    mesh::mesh_chunk_surface,
    world::{Chunk, World, SECTION_SIZE},
    Vertex,
};

const TARGET: u32 = 64;

/// Positions-only shader matching `square.wgsl`'s vertex stage; the fragment stage just marks
/// coverage, since this test is about which triangles survive culling, not what they look like.
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

/// Camera-relative view + perspective, exactly as `Camera::build_view_projection_matrix` builds
/// it. Duplicated rather than driven through `Camera` because `Camera` has no "look at an
/// arbitrary point" constructor, and the property under test is the matrix's handedness.
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

/// Renders the quads of a solid chunk with backface culling on, and returns how many pixels
/// were covered. `None` when no GPU adapter is available.
fn covered_pixels(eye: [f32; 3], target: [f32; 3]) -> Option<u32> {
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
    let meshes = mesh_chunk_surface(&world, (0, 0));
    let mut vertices: Vec<Vertex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    for (_, mesh) in &meshes {
        let base = vertices.len() as u32;
        vertices.extend_from_slice(mesh.vertices());
        indices.extend(mesh.indices().iter().map(|i| i + base));
    }
    assert!(!indices.is_empty(), "solid chunk must produce geometry");

    let mut uniform = [0f32; 20];
    uniform[..16].copy_from_slice(AsRef::<[f32; 16]>::as_ref(&view_proj(eye, target)));
    uniform[16..19].copy_from_slice(&eye);

    let uniform_buffer = wgpu_buffer(&device, bytemuck::cast_slice(&uniform), wgpu::BufferUsages::UNIFORM);
    let vertex_buffer = wgpu_buffer(&device, bytemuck::cast_slice(&vertices), wgpu::BufferUsages::VERTEX);
    let index_buffer = wgpu_buffer(&device, bytemuck::cast_slice(&indices), wgpu::BufferUsages::INDEX);

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
        // The property under test.
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
        size: wgpu::Extent3d { width: TARGET, height: TARGET, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    // 64 px * 4 bytes = 256, already the required copy alignment.
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
        wgpu::Extent3d { width: TARGET, height: TARGET, depth_or_array_layers: 1 },
    );
    queue.submit(Some(encoder.finish()));

    readback.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::Wait);
    let data = readback.slice(..).get_mapped_range();
    let lit = data.chunks_exact(4).filter(|px| px[0] > 128).count() as u32;
    drop(data);
    readback.unmap();
    Some(lit)
}

fn wgpu_buffer(device: &wgpu::Device, contents: &[u8], usage: wgpu::BufferUsages) -> wgpu::Buffer {
    use wgpu::util::DeviceExt;
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents,
        usage,
    })
}

#[test]
fn outward_faces_survive_backface_culling() {
    // Looking at the chunk's +X side from outside it.
    let Some(outside) = covered_pixels([40.0, 8.0, 8.0], [8.0, 8.0, 8.0]) else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    assert!(
        outside > 500,
        "a solid chunk viewed from outside should cover the view, got {outside} px"
    );
}

#[test]
fn inward_faces_are_culled() {
    // Same geometry, camera inside the solid block looking out: every face now presents its
    // back, so backface culling should leave the frame empty. If the winding were inverted
    // this is the assertion that fails.
    let Some(inside) = covered_pixels([8.0, 8.0, 8.0], [40.0, 8.0, 8.0]) else {
        eprintln!("no GPU adapter; skipping");
        return;
    };
    assert_eq!(
        inside, 0,
        "faces seen from behind must be culled, got {inside} px"
    );
}
