use std::sync::Arc;

use bytemuck::Zeroable;
use pollster::FutureExt;
use std::borrow::Cow;
use wgpu::{
    BindGroup, FragmentState,
    util::{BufferInitDescriptor, DeviceExt},
    wgc::{binding_model::BindGroupLayoutDescriptor, id::markers::BindGroupLayout, pipeline},
    wgt::TextureDescriptor,
};
use winit::{
    application::ApplicationHandler,
    event::*,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{CursorGrabMode, Fullscreen, Window, WindowAttributes, WindowId},
};

mod camera;
mod mesh;

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
    color: [f32; 3],
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
                format: wgpu::VertexFormat::Float32x3,
            },
        ],
    };
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Zeroable, bytemuck::Pod)]
struct Uniform_Camera {
    fields: [[f32; 4]; 4],
}

impl Uniform_Camera {
    fn new() -> Self {
        Self {
            fields: [[0.0; 4]; 4],
        }
    }

    fn update(&mut self, camera: &camera::Camera) {
        self.fields = camera.build_view_projection_matrix().into();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockType {
    AIR,
    STONE,
    GRASS,
}

const VERTICES: &[Vertex] = &[
    Vertex {
        position: [0.0, 0.0, 0.0],
        color: [1.0, 0.0, 0.0],
    },
    Vertex {
        position: [1.0, 0.0, 0.0],
        color: [1.0, 0.0, 0.0],
    },
    Vertex {
        position: [1.0, 0.0, 1.0],
        color: [1.0, 0.0, 0.0],
    },
    Vertex {
        position: [0.0, 0.0, 1.0],
        color: [1.0, 0.0, 0.0],
    },
    Vertex {
        position: [0.0, 1.0, 0.0],
        color: [1.0, 0.0, 0.0],
    },
    Vertex {
        position: [1.0, 1.0, 0.0],
        color: [1.0, 0.0, 0.0],
    },
    Vertex {
        position: [1.0, 1.0, 1.0],
        color: [1.0, 0.0, 1.0],
    },
    Vertex {
        position: [0.0, 1.0, 1.0],
        color: [1.0, 0.0, 0.0],
    },
];

const INDICES: &[u16] = &[
    // // Bottom face (facing negative Y)
    0, 1, 2, // Triangle 1
    2, 3, 0, // Triangle 2
    // // Top face (facing positive Y)
    4, 7, 6, // Triangle 1
    6, 5, 4, // Triangle 2
    // // Right face (facing positive Z)
    2, 6, 7, // Triangle 1
    7, 3, 2, // Triangle 2
    // // Left face (facing negative Z)
    0, 4, 5, // Triangle 1
    5, 1, 0, // Triangle 2
    // // Front face (facing negative X)
    0, 3, 7, // Triangle 1
    7, 4, 0, // Triangle 2
    // // Bottom face (facing positive X)
    1, 5, 6, // Triangle 1
    6, 2, 1, // Triangle 2
];
struct State {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    render_pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    num_indices: u32,
    camera: camera::Camera,
    camera_uniform: Uniform_Camera,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    camera_controller: camera::Controller,
    is_surface_configured: bool,
    delta: u128,
    last_frame_time: std::time::Instant,
    depth_texture: wgpu::TextureView,
}

impl State {
    async fn new(window: Arc<Window>) -> Self {
        let size = window.inner_size();
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let surface = instance.create_surface(window.clone()).unwrap();

        window.set_cursor_grab(winit::window::CursorGrabMode::Locked);
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
                required_features: wgpu::Features::POLYGON_MODE_LINE,
                ..Default::default()
            })
            .await
            .unwrap();

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: wgpu::TextureFormat::Bgra8Unorm,
            width: size.width,
            height: size.height,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        let camera =
            camera::Camera::new(config.width as f32 / config.height as f32, 45.0, 0.1, 100.0);

        let mut camera_uniform = Uniform_Camera::new();
        camera_uniform.update(&camera);

        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Camera Buffer"),
            contents: bytemuck::cast_slice(&[camera_uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let camera_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Camera Bind Group Layout"),
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

        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Camera Bind Group"),
            layout: &camera_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });

        // Bind group can be used once you have a camera to render the 3D scene since you can use that data in the wgsl shader
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Pipeline Layout"),
            bind_group_layouts: &[&camera_bind_group_layout],
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

        // Change this to create_buffer() once you have dynamic data
        let vertex_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("Vertex Buffer"),
            contents: bytemuck::cast_slice(VERTICES),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let index_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("Index Buffer"),
            contents: bytemuck::cast_slice(INDICES),
            usage: wgpu::BufferUsages::INDEX,
        });

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

        let num_indices = INDICES.len() as u32;

        let camera_controller = camera::Controller::new(1, 0.00001);

        return Self {
            window,
            surface: surface,
            device,
            queue,
            config,
            vertex_buffer,
            index_buffer,
            render_pipeline,
            num_indices,
            camera,
            camera_uniform,
            camera_buffer,
            camera_bind_group,
            camera_controller,
            is_surface_configured: false,
            last_frame_time: std::time::Instant::now(),
            delta: 0,
            depth_texture: depth_texture_view,
        };
    }

    fn update(&mut self) {
        self.camera_uniform.update(&self.camera);
        self.queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::cast_slice(&[self.camera_uniform]),
        );
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

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("render pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
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
            render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            render_pass.draw_indexed(0..self.num_indices, 0, 0..1);
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
        let window = Arc::new(
            event_loop
                .create_window(
                    WindowAttributes::default()
                        .with_title("Voxel Engine")
                        .with_visible(false),
                )
                .unwrap(),
        );
        let mut state = pollster::block_on(State::new(window));
        let window = Arc::clone(&state.window);
        state.window.set_maximized(true);
        let size = state.window.inner_size();
        state.resize(size.width, size.height);
        state.update();
        state.render().unwrap();
        window.set_visible(true);
        self.state = Some(state);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
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
                        physical_key: keyCode,
                        repeat: false,
                        ..
                    },
                ..
            } => {
                if let Some(state) = self.state.as_mut() {
                    match keyCode {
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
                            let _ = state.window.set_cursor_grab(CursorGrabMode::None);
                            state.window.set_cursor_visible(true);
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
        event_loop: &ActiveEventLoop,
        device_id: DeviceId,
        event: DeviceEvent,
    ) {
        match event {
            DeviceEvent::MouseMotion { delta } => {
                if let Some(state) = self.state.as_mut() {
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

fn main() {
    pollster::block_on(run());
}

async fn run() {
    let event_loop = EventLoop::new().unwrap();

    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = App::new();
    let _ = event_loop.run_app(&mut app);
}
