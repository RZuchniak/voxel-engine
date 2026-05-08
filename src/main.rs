use std::{
    borrow::Cow,
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
};

use cgmath::{EuclideanSpace, InnerSpace};
use wgpu::{FragmentState, util::DeviceExt};
#[cfg(target_arch = "wasm32")]
use winit::platform::web::WindowAttributesExtWebSys;
use winit::{
    application::ApplicationHandler,
    event::*,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{CursorGrabMode, Window, WindowAttributes, WindowId},
};

mod block;
mod camera;
mod mesh;
mod source;
mod streamer;
mod texture;
mod world;

#[cfg(not(target_arch = "wasm32"))]
use source::AnvilSource;
#[cfg(target_arch = "wasm32")]
use source::PackedWebSource;
use streamer::ChunkStreamer;
use world::{MIN_SECTION_Y, SECTION_SIZE, World};

const LOAD_DISTANCE_CHUNKS: i32 = 8;
const UNLOAD_MARGIN_CHUNKS: i32 = 2;
const REMESH_BUDGET_PER_FRAME: usize = 1;

#[rustfmt::skip]
pub const OPENGL_TO_WGPU_MATRIX: cgmath::Matrix4<f32> = cgmath::Matrix4::from_cols(
    cgmath::Vector4::new(1.0, 0.0, 0.0, 0.0),
    cgmath::Vector4::new(0.0, 1.0, 0.0, 0.0),
    cgmath::Vector4::new(0.0, 0.0, 0.5, 0.0),
    cgmath::Vector4::new(0.0, 0.0, 0.5, 1.0),
);

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 3],
    uv: [f32; 2],
    tex_layer: u32,
    light: u32,
}

impl Vertex {
    pub const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
            wgpu::VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x3,
            },
            wgpu::VertexAttribute {
                offset: std::mem::size_of::<[f32; 3]>() as wgpu::BufferAddress,
                shader_location: 1,
                format: wgpu::VertexFormat::Float32x2,
            },
            wgpu::VertexAttribute {
                offset: (std::mem::size_of::<[f32; 3]>() + std::mem::size_of::<[f32; 2]>())
                    as wgpu::BufferAddress,
                shader_location: 2,
                format: wgpu::VertexFormat::Uint32,
            },
            wgpu::VertexAttribute {
                offset: (std::mem::size_of::<[f32; 3]>()
                    + std::mem::size_of::<[f32; 2]>()
                    + std::mem::size_of::<u32>()) as wgpu::BufferAddress,
                shader_location: 3,
                format: wgpu::VertexFormat::Uint32,
            },
        ],
    };
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Zeroable, bytemuck::Pod)]
struct UniformCamera {
    fields: [[f32; 4]; 4],
}

impl UniformCamera {
    fn new() -> Self {
        Self {
            fields: [[0.0; 4]; 4],
        }
    }

    fn update(&mut self, camera: &camera::Camera) {
        self.fields = camera.build_view_projection_matrix().into();
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Zeroable, bytemuck::Pod)]
struct UniformSky {
    top_color: [f32; 4],
    horizon_color: [f32; 4],
    fog_params: [f32; 4], // start_distance, end_distance, strength, _pad
    camera_pos: [f32; 4],
}

impl UniformSky {
    fn new() -> Self {
        Self {
            top_color: [0.42, 0.64, 0.90, 1.0],
            horizon_color: [0.72, 0.84, 0.98, 1.0],
            // World-space fog distances in block units.
            fog_params: [160.0, 430.0, 0.22, 0.0],
            camera_pos: [0.0, 0.0, 0.0, 0.0],
        }
    }
}

struct SectionMesh {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    num_indices: u32,
}

struct State {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    render_pipeline: wgpu::RenderPipeline,
    _world: World,
    streamer: ChunkStreamer,
    section_meshes: HashMap<(i32, i32, i32), SectionMesh>,
    remesh_queue: VecDeque<(i32, i32)>,
    remesh_pending: HashSet<(i32, i32)>,
    camera: camera::Camera,
    camera_uniform: UniformCamera,
    sky_uniform: UniformSky,
    camera_buffer: wgpu::Buffer,
    sky_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    texture_bind_group: wgpu::BindGroup,
    camera_controller: camera::Controller,
    mouse_captured: bool,
    is_surface_configured: bool,
    delta: u128,
    last_frame_time: std::time::Instant,
    depth_texture: wgpu::TextureView,
    visible_sections_last_frame: usize,
    uploaded_meshes_last_frame: usize,
    profile_accum: u128,
    render_frame_index: u64,
}

impl State {
    fn section_detail_ring(&self, section_coord: (i32, i32, i32)) -> u8 {
        let camera_pos = self.camera.position();
        let center = cgmath::Vector3::new(
            section_coord.0 as f32 * SECTION_SIZE as f32 + 8.0,
            section_coord.1 as f32 + 8.0,
            section_coord.2 as f32 * SECTION_SIZE as f32 + 8.0,
        );
        let dist = (center - camera_pos.to_vec()).magnitude();
        let chunk_dist = dist / SECTION_SIZE as f32;
        if chunk_dist <= 7.5 {
            0 // near
        } else if chunk_dist <= 11.5 {
            1 // mid
        } else if chunk_dist <= 15.0 {
            2 // far
        } else {
            3 // culled
        }
    }

    fn section_visible(&self, section_coord: (i32, i32, i32)) -> bool {
        let camera_pos = self.camera.position();
        let forward = self.camera.forward();
        let center = cgmath::Vector3::new(
            section_coord.0 as f32 * SECTION_SIZE as f32 + 8.0,
            section_coord.1 as f32 + 8.0,
            section_coord.2 as f32 * SECTION_SIZE as f32 + 8.0,
        );
        let to_section = center - camera_pos.to_vec();
        let dist2 = to_section.magnitude2();

        // Hard distance cap to avoid drawing far sections.
        let max_distance = SECTION_SIZE as f32 * 15.0;
        if dist2 > max_distance * max_distance {
            return false;
        }

        // Backface cone-ish culling: keep some leeway to avoid popping near edges.
        let dir = to_section.normalize();
        dir.dot(forward) > -0.25
    }

    async fn new(window: Arc<Window>) -> Self {
        let size = window.inner_size();
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let surface = instance.create_surface(window.clone()).unwrap();

        let _ = window.set_cursor_grab(winit::window::CursorGrabMode::Locked);
        window.set_cursor_visible(false);

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
            })
            .await
            .unwrap();

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_features: wgpu::Features::empty(),
                ..Default::default()
            })
            .await
            .unwrap();

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .unwrap_or(surface_caps.formats[0]);
        let present_mode = if surface_caps.present_modes.contains(&wgpu::PresentMode::Fifo) {
            wgpu::PresentMode::Fifo
        } else {
            surface_caps.present_modes[0]
        };
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width,
            height: size.height,
            present_mode,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        let camera =
            camera::Camera::new(config.width as f32 / config.height as f32, 45.0, 0.1, 500.0);

        let mut camera_uniform = UniformCamera::new();
        camera_uniform.update(&camera);

        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Camera Buffer"),
            contents: bytemuck::cast_slice(&[camera_uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let sky_uniform = UniformSky::new();
        let sky_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Sky Buffer"),
            contents: bytemuck::cast_slice(&[sky_uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let camera_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Camera Bind Group Layout"),
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
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Camera Bind Group"),
            layout: &camera_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: sky_buffer.as_entire_binding(),
                },
            ],
        });

        // Bind group can be used once you have a camera to render the 3D scene since you can use that data in the wgsl shader
        let textures = texture::create_block_textures(&device, &queue);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Pipeline Layout"),
            bind_group_layouts: &[&camera_bind_group_layout, &textures.bind_group_layout],
            push_constant_ranges: &[],
        });

        surface.configure(&device, &config);

        // Shader
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shader"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("square.wgsl"))),
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Vertex::LAYOUT],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                polygon_mode: wgpu::PolygonMode::Fill,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let world = World::new();
        #[cfg(not(target_arch = "wasm32"))]
        let world_source: Arc<dyn source::WorldSource> =
            Arc::new(AnvilSource::new("saves/Basic_World"));
        #[cfg(target_arch = "wasm32")]
        let world_source: Arc<dyn source::WorldSource> = Arc::new(PackedWebSource::new("world"));
        let streamer = ChunkStreamer::new(world_source);
        let section_meshes = HashMap::new();
        let remesh_queue = VecDeque::new();
        let remesh_pending = HashSet::new();

        let depth_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Depth Texture"),
            size: wgpu::Extent3d {
                width: config.width,
                height: config.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[wgpu::TextureFormat::Depth32Float],
        });

        let depth_texture_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let camera_controller = camera::Controller::new(20.0, 0.002);

        return Self {
            window,
            surface: surface,
            device,
            queue,
            config,
            _world: world,
            streamer,
            section_meshes,
            remesh_queue,
            remesh_pending,
            render_pipeline,
            camera,
            camera_uniform,
            sky_uniform,
            camera_buffer,
            sky_buffer,
            camera_bind_group,
            texture_bind_group: textures.bind_group,
            camera_controller,
            mouse_captured: true,
            is_surface_configured: false,
            last_frame_time: std::time::Instant::now(),
            delta: 0,
            depth_texture: depth_texture_view,
            visible_sections_last_frame: 0,
            uploaded_meshes_last_frame: 0,
            profile_accum: 0,
            render_frame_index: 0,
        };
    }

    fn update(&mut self) {
        self.update_streaming();
        self.camera_uniform.update(&self.camera);
        self.queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::cast_slice(&[self.camera_uniform]),
        );
        let cp = self.camera.position();
        self.sky_uniform.camera_pos = [cp.x, cp.y, cp.z, 0.0];
        self.queue
            .write_buffer(&self.sky_buffer, 0, bytemuck::cast_slice(&[self.sky_uniform]));
        self.profile_accum += self.delta;
        if self.profile_accum > 1_000_000 {
            println!(
                "profile visible_sections={} uploaded_meshes={} loaded_chunks={}",
                self.visible_sections_last_frame,
                self.uploaded_meshes_last_frame,
                self._world.chunks().count()
            );
            self.profile_accum = 0;
        }
    }

    fn update_streaming(&mut self) {
        let mesh_upload_budget = 10usize;
        let player_chunk_x = (self.camera.position().x.floor() as i32).div_euclid(SECTION_SIZE as i32);
        let player_chunk_z = (self.camera.position().z.floor() as i32).div_euclid(SECTION_SIZE as i32);

        let mut desired = Vec::new();
        for dz in -LOAD_DISTANCE_CHUNKS..=LOAD_DISTANCE_CHUNKS {
            for dx in -LOAD_DISTANCE_CHUNKS..=LOAD_DISTANCE_CHUNKS {
                let cx = player_chunk_x + dx;
                let cz = player_chunk_z + dz;
                let dist2 = dx * dx + dz * dz;
                desired.push(((cx, cz), dist2));
            }
        }
        desired.sort_by_key(|(_, dist2)| *dist2);

        for (coord, _) in desired.iter().copied() {
            if !self._world.has_chunk(coord) && !self.streamer.is_in_flight(coord) {
                self.streamer.request_chunk(coord);
            }
        }

        let loaded: Vec<(i32, i32)> = self._world.chunks().map(|c| c.coord()).collect();
        for coord in loaded {
            let dx = coord.0 - player_chunk_x;
            let dz = coord.1 - player_chunk_z;
            if dx.abs() > LOAD_DISTANCE_CHUNKS + UNLOAD_MARGIN_CHUNKS
                || dz.abs() > LOAD_DISTANCE_CHUNKS + UNLOAD_MARGIN_CHUNKS
            {
                self._world.remove_chunk(coord);
                self.section_meshes
                    .retain(|(chunk_x, _, chunk_z), _| *chunk_x != coord.0 || *chunk_z != coord.1);
            }
        }

        let ready = self.streamer.poll_ready(mesh_upload_budget);
        self.uploaded_meshes_last_frame = ready.len();
        for meshed in ready {
            let coord = meshed.coord;
            self._world.insert_chunk(meshed.chunk);
            self.upload_chunk_meshes(coord, meshed.section_meshes);

            // Rebuild the arriving chunk and orthogonal neighbors so border faces are culled
            // correctly once adjacent chunks become available.
            let remesh_targets = [
                coord,
                (coord.0 + 1, coord.1),
                (coord.0 - 1, coord.1),
                (coord.0, coord.1 + 1),
                (coord.0, coord.1 - 1),
            ];
            for target in remesh_targets {
                if self._world.has_chunk(target) {
                    self.enqueue_remesh(target);
                }
            }
        }

        // More aggressive remesh while close chunks are changing, gentler farther away.
        for _ in 0..(REMESH_BUDGET_PER_FRAME + 1) {
            let Some(coord) = self.remesh_queue.pop_front() else {
                break;
            };
            self.remesh_pending.remove(&coord);
            if self._world.has_chunk(coord) {
                self.rebuild_chunk_meshes(coord);
            }
        }
    }

    fn enqueue_remesh(&mut self, coord: (i32, i32)) {
        if self.remesh_pending.insert(coord) {
            self.remesh_queue.push_back(coord);
        }
    }

    fn upload_chunk_meshes(&mut self, coord: (i32, i32), section_meshes: Vec<(usize, mesh::MeshData)>) {
        self.section_meshes
            .retain(|(chunk_x, _, chunk_z), _| *chunk_x != coord.0 || *chunk_z != coord.1);
        for (section_index, mesh_data) in section_meshes {
            let vertex_buffer = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Section Vertex Buffer"),
                    contents: bytemuck::cast_slice(mesh_data.vertices()),
                    usage: wgpu::BufferUsages::VERTEX,
                });
            let index_buffer = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Section Index Buffer"),
                    contents: bytemuck::cast_slice(mesh_data.indices()),
                    usage: wgpu::BufferUsages::INDEX,
                });
            let section_world_y = (section_index as i32 + MIN_SECTION_Y) * SECTION_SIZE as i32;
            self.section_meshes.insert(
                (coord.0, section_world_y, coord.1),
                SectionMesh {
                    vertex_buffer,
                    index_buffer,
                    num_indices: mesh_data.indices().len() as u32,
                },
            );
        }
    }

    fn rebuild_chunk_meshes(&mut self, coord: (i32, i32)) {
        self.section_meshes
            .retain(|(chunk_x, _, chunk_z), _| *chunk_x != coord.0 || *chunk_z != coord.1);
        let Some(chunk) = self._world.chunk(coord) else {
            return;
        };

        let mut rebuilt = Vec::new();
        for section_index in chunk.populated_section_indices() {
            let Some(mesh_data) = mesh::mesh_section(&self._world, coord, section_index) else {
                continue;
            };
            rebuilt.push((section_index, mesh_data));
        }

        for (section_index, mesh_data) in rebuilt {
            let vertex_buffer = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Section Vertex Buffer"),
                    contents: bytemuck::cast_slice(mesh_data.vertices()),
                    usage: wgpu::BufferUsages::VERTEX,
                });
            let index_buffer = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Section Index Buffer"),
                    contents: bytemuck::cast_slice(mesh_data.indices()),
                    usage: wgpu::BufferUsages::INDEX,
                });
            let section_world_y = (section_index as i32 + MIN_SECTION_Y) * SECTION_SIZE as i32;
            self.section_meshes.insert(
                (coord.0, section_world_y, coord.1),
                SectionMesh {
                    vertex_buffer,
                    index_buffer,
                    num_indices: mesh_data.indices().len() as u32,
                },
            );
        }
    }

    fn set_mouse_capture(&mut self, captured: bool) {
        self.mouse_captured = captured;
        if captured {
            let _ = self.window.set_cursor_grab(CursorGrabMode::Locked);
            self.window.set_cursor_visible(false);
        } else {
            let _ = self.window.set_cursor_grab(CursorGrabMode::None);
            self.window.set_cursor_visible(true);
            self.camera_controller.mouse_delta = (0.0, 0.0);
        }
    }

    fn render(&mut self) -> Result<(), wgpu::SurfaceError> {
        if !self.is_surface_configured {
            return Err(wgpu::SurfaceError::Lost);
        }

        let frame = match self.surface.get_current_texture() {
            Ok(frame) => frame,
            Err(err) => {
                return Err(err);
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Render Encoder"),
            });

        self.render_frame_index = self.render_frame_index.wrapping_add(1);
        self.visible_sections_last_frame = 0;
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("render pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // Temporary sky-like clear color while full skybox rendering is not wired.
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: self.sky_uniform.horizon_color[0] as f64,
                            g: self.sky_uniform.horizon_color[1] as f64,
                            b: self.sky_uniform.horizon_color[2] as f64,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_texture,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            render_pass.set_pipeline(&self.render_pipeline);
            render_pass.set_bind_group(0, &self.camera_bind_group, &[]);
            render_pass.set_bind_group(1, &self.texture_bind_group, &[]);
            for (coord, section) in &self.section_meshes {
                if !self.section_visible(*coord) {
                    continue;
                }
                let _ring = self.section_detail_ring(*coord);
                self.visible_sections_last_frame += 1;
                render_pass.set_vertex_buffer(0, section.vertex_buffer.slice(..));
                render_pass
                    .set_index_buffer(section.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                render_pass.draw_indexed(0..section.num_indices, 0, 0..1);
            }
        }

        self.queue.submit(Some(encoder.finish()));
        frame.present();
        Ok(())
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.config.width = width;
            self.config.height = height;
            self.camera.aspect_ratio = width as f32 / height as f32;
            self.surface.configure(&self.device, &self.config);

            // Recreate depth texture
            let size = wgpu::Extent3d {
                width: self.config.width,
                height: self.config.height,
                depth_or_array_layers: 1,
            };
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Depth Texture"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[wgpu::TextureFormat::Depth32Float],
            });
            self.depth_texture = texture.create_view(&wgpu::TextureViewDescriptor::default());

            self.is_surface_configured = true;
        }
    }
}

struct App {
    state: Option<State>,
}

impl App {
    fn new() -> Self {
        Self { state: None }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        #[cfg(target_arch = "wasm32")]
        let attributes = {
            use wasm_bindgen::JsCast;
            let window = web_sys::window().expect("web window");
            let document = window.document().expect("document");
            let canvas = document
                .get_element_by_id("voxel-canvas")
                .and_then(|el| el.dyn_into::<web_sys::HtmlCanvasElement>().ok());
            WindowAttributes::default()
                .with_title("Voxel Engine")
                .with_canvas(canvas)
                .with_visible(true)
        };
        #[cfg(not(target_arch = "wasm32"))]
        let attributes = WindowAttributes::default()
            .with_title("Voxel Engine")
            .with_visible(false);
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .unwrap(),
        );
        let mut state = pollster::block_on(State::new(window));
        #[cfg(not(target_arch = "wasm32"))]
        let window = Arc::clone(&state.window);
        #[cfg(not(target_arch = "wasm32"))]
        state.window.set_maximized(true);
        let size = state.window.inner_size();
        state.resize(size.width, size.height);
        state.update();
        state.render().unwrap();
        #[cfg(not(target_arch = "wasm32"))]
        window.set_visible(true);
        self.state = Some(state);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::RedrawRequested => {
                if let Some(state) = self.state.as_mut() {
                    let current_time = std::time::Instant::now();
                    state.delta = current_time
                        .duration_since(state.last_frame_time)
                        .as_micros();
                    state.last_frame_time = current_time;
                    state
                        .camera_controller
                        .update(state.delta, &mut state.camera);
                    state.update();
                    state.window.request_redraw();
                    match state.render() {
                        Ok(_) => {
                            //println!("Frame rate: {}", 1.0 / state.delta as f64 * 1000000.0);
                        }
                        Err(_) => {
                            let size = state.window.inner_size();
                            state.resize(size.width, size.height);
                        }
                    }
                }
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        state: key_state,
                        physical_key: key_code,
                        repeat: false,
                        ..
                    },
                ..
            } => {
                if let Some(state) = self.state.as_mut() {
                    match key_code {
                        PhysicalKey::Code(KeyCode::KeyW) => {
                            state.camera_controller.forward = key_state == ElementState::Pressed;
                        }
                        PhysicalKey::Code(KeyCode::KeyS) => {
                            state.camera_controller.backward = key_state == ElementState::Pressed;
                        }
                        PhysicalKey::Code(KeyCode::KeyA) => {
                            state.camera_controller.left = key_state == ElementState::Pressed;
                        }
                        PhysicalKey::Code(KeyCode::KeyD) => {
                            state.camera_controller.right = key_state == ElementState::Pressed;
                        }
                        PhysicalKey::Code(KeyCode::Space) => {
                            state.camera_controller.up = key_state == ElementState::Pressed;
                        }
                        PhysicalKey::Code(KeyCode::ShiftLeft) => {
                            state.camera_controller.down = key_state == ElementState::Pressed;
                        }
                        PhysicalKey::Code(KeyCode::KeyP) => {
                            if key_state == ElementState::Pressed {
                                state.set_mouse_capture(!state.mouse_captured);
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: DeviceId,
        event: DeviceEvent,
    ) {
        match event {
            DeviceEvent::MouseMotion { delta } => {
                if let Some(state) = self.state.as_mut() {
                    if !state.mouse_captured {
                        return;
                    }
                    let new_delta = (
                        delta.0 + state.camera_controller.mouse_delta.0,
                        delta.1 + state.camera_controller.mouse_delta.1,
                    );
                    state.camera_controller.mouse_delta = new_delta;
                }
            }
            _ => {}
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    pollster::block_on(run());
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn wasm_start() {
    wasm_bindgen_futures::spawn_local(async {
        run().await;
    });
}

#[cfg(target_arch = "wasm32")]
fn main() {}

async fn run() {
    let event_loop = EventLoop::new().unwrap();

    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = App::new();
    let _ = event_loop.run_app(&mut app);
}
