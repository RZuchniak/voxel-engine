use std::{
    borrow::Cow,
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
};
#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;

use cgmath::InnerSpace;
use wgpu::{FragmentState, util::DeviceExt};
use web_time::Instant;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;
#[cfg(target_arch = "wasm32")]
use winit::platform::web::WindowAttributesExtWebSys;
use winit::{
    application::ApplicationHandler,
    event::*,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{CursorGrabMode, Window, WindowAttributes, WindowId},
};

mod streamer;
mod hud;
#[cfg(target_arch = "wasm32")]
mod web_api;
#[cfg(target_arch = "wasm32")]
use voxel_engine::worker_protocol;
#[cfg(target_arch = "wasm32")]
mod chunk_worker;
#[cfg(target_arch = "wasm32")]
mod worker_api;
#[cfg(target_arch = "wasm32")]
mod worker_bridge;

// Re-bind lib modules into the bin's root namespace so existing `crate::…` paths in
// submodules (e.g. `terrain.rs` -> `crate::noise`) keep resolving as modules migrate.
use voxel_engine::{
    block, camera, cull, mesh, noise, platform, render, source, terrain, texture, visibility,
    world,
};
use voxel_engine::{OPENGL_TO_WGPU_MATRIX, Vertex};

#[cfg(not(target_arch = "wasm32"))]
use voxel_engine::source::AnvilSource;
use streamer::ChunkStreamer;
use voxel_engine::world::{MIN_SECTION_Y, SECTION_COUNT, SECTION_SIZE, World};

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
            // Runtime-updated from active section draw distance (start, end, strength, _pad).
            fog_params: [200.0, 480.0, 1.0, 0.0],
            camera_pos: [0.0, 0.0, 0.0, 0.0],
        }
    }
}

/// Streamer results waiting to be applied, plus an index of the coords whose chunk data is
/// queued here.
///
/// The index is load-bearing, not a convenience. A chunk that has finished generating but has
/// not been applied yet is in neither `_world` nor the streamer's in-flight set, so without it
/// `update_streaming` re-requests every chunk sitting in this queue on every frame — the pool
/// regenerates them and the main thread re-meshes them on pop. Measured at loading radius 12:
/// 4492 requests and 3479 mesh jobs to build 625 chunks.
struct PendingReady {
    queue: VecDeque<streamer::MeshedChunk>,
    queued_loads: HashSet<(i32, i32)>,
}

impl PendingReady {
    fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            queued_loads: HashSet::new(),
        }
    }

    fn len(&self) -> usize {
        self.queue.len()
    }

    /// Finished chunk loads waiting to be applied. See [`platform::MAX_PENDING_LOAD_BACKLOG`].
    fn load_backlog(&self) -> usize {
        self.queued_loads.len()
    }

    /// True when this coord's *chunk data* is queued. Remesh results carry no chunk data and
    /// are deliberately not indexed: their coord is already in the world.
    fn contains_load(&self, coord: (i32, i32)) -> bool {
        self.queued_loads.contains(&coord)
    }

    /// True when any result for this coord is queued, load or remesh.
    fn contains_any(&self, coord: (i32, i32)) -> bool {
        self.queue.iter().any(|meshed| meshed.coord == coord)
    }

    fn index(&mut self, meshed: &streamer::MeshedChunk) {
        if !meshed.is_remesh {
            self.queued_loads.insert(meshed.coord);
        }
    }

    fn unindex(&mut self, meshed: &streamer::MeshedChunk) {
        if !meshed.is_remesh {
            self.queued_loads.remove(&meshed.coord);
        }
    }

    fn push_front(&mut self, meshed: streamer::MeshedChunk) {
        self.index(&meshed);
        self.queue.push_front(meshed);
    }

    fn push_back(&mut self, meshed: streamer::MeshedChunk) {
        self.index(&meshed);
        self.queue.push_back(meshed);
    }

    fn pop_front(&mut self) -> Option<streamer::MeshedChunk> {
        let meshed = self.queue.pop_front()?;
        self.unindex(&meshed);
        Some(meshed)
    }

    fn retain(&mut self, keep: impl Fn(&streamer::MeshedChunk) -> bool) {
        let queued_loads = &mut self.queued_loads;
        self.queue.retain(|meshed| {
            let keeping = keep(meshed);
            if !keeping && !meshed.is_remesh {
                queued_loads.remove(&meshed.coord);
            }
            keeping
        });
    }

    /// Drop the rearmost completed load. Remesh results are never dropped: the streamer has
    /// already cleared `remesh_in_flight`, so losing one leaves stale chunk-border geometry
    /// forever. Returns false when the queue holds nothing but remeshes.
    fn drop_last_load(&mut self) -> bool {
        let Some(i) = self.queue.iter().rposition(|meshed| !meshed.is_remesh) else {
            return false;
        };
        if let Some(meshed) = self.queue.remove(i) {
            self.unindex(&meshed);
        }
        true
    }
}

struct SectionDrawRange {
    first_index: u32,
    index_count: u32,
    section_world_y: i32,
}

struct ChunkGpuMesh {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    section_draws: Vec<SectionDrawRange>,
    vertex_capacity: u64,
    index_capacity: u64,
    bounds_min: cgmath::Vector3<f32>,
    bounds_max: cgmath::Vector3<f32>,
    surface_max_y: i32,
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
    pending_ready: PendingReady,
    chunk_meshes: HashMap<(i32, i32), ChunkGpuMesh>,
    remesh_priority: VecDeque<(i32, i32)>,
    remesh_neighbor: VecDeque<(i32, i32)>,
    remesh_priority_pending: HashSet<(i32, i32)>,
    remesh_neighbor_pending: HashSet<(i32, i32)>,
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
    last_frame_time: Instant,
    depth_texture: wgpu::TextureView,
    visible_draw_calls_last_frame: usize,
    visible_chunks_last_frame: usize,
    uploaded_meshes_last_frame: usize,
    profile_accum: u128,
    render_frame_index: u64,
    hud: hud::HudOverlay,
    fps_ema: f32,
    section_draw_distance_chunks: f32,
    /// False until the neighborhood around the spawn has GPU meshes and no pending mesh work.
    world_ready: bool,
    /// When the engine started, for the load-timeline profile line.
    started_at: web_time::Instant,
    /// Loading-radius tiles already known finished. See [`Self::bootstrap_ready_tiles_cached`].
    bootstrap_ready_cache: HashSet<(i32, i32)>,
    /// Drain composition, for the load profile line.
    drained_new_last_frame: usize,
    drained_remesh_last_frame: usize,
    /// Microseconds spent in each phase of the last update(), for the load profile line.
    phase_stream_us: u128,
    phase_mesh_us: u128,
    /// Bootstrap tiles we meshed (including empty/culled) so readiness does not stall forever.
    bootstrap_mesh_attempted: HashSet<(i32, i32)>,
    /// Cumulative chunk loads requested from the streamer, for the load profile line.
    chunk_requests_issued: u64,
    /// Cumulative chunks meshed, for the load profile line.
    chunks_meshed: u64,
    /// Per-chunk section connectivity, delivered with each mesh result. Chunks absent here are
    /// treated as fully transparent by the traversal — see [`cull::SectionGraph::walk`].
    chunk_visibility: HashMap<(i32, i32), [visibility::VisibilitySet; SECTION_COUNT]>,
    /// Reused across frames; owns the traversal's scratch buffers.
    section_graph: cull::SectionGraph,
    /// Sections the traversal reached last frame, for the profile line.
    reachable_sections_last_frame: usize,
    /// Seed-mode world: chunks are generated rather than read from a save, which is far more
    /// expensive per chunk and gets larger streaming budgets.
    procedural_world: bool,
}

impl State {
    #[inline]
    fn round_mesh_buffer_capacity(need: u64) -> u64 {
        if need == 0 {
            return 256;
        }
        need.max(256).next_power_of_two()
    }

    /// The chunk plus its four neighbours — everything `mesh_chunk_surface` can reach when it
    /// tests a block's neighbour across a chunk border.
    fn mesh_snapshot(world: &World, coord: (i32, i32)) -> World {
        let mut local_world = World::new();
        for c in [
            coord,
            (coord.0 + 1, coord.1),
            (coord.0 - 1, coord.1),
            (coord.0, coord.1 + 1),
            (coord.0, coord.1 - 1),
        ] {
            if let Some(chunk) = world.chunk_shared(c) {
                local_world.insert_shared(chunk);
            }
        }
        local_world
    }

    /// High-priority remesh (arriving chunk). Supersedes a pending neighbor job for the same coord.
    fn enqueue_priority_remesh(&mut self, coord: (i32, i32)) {
        self.remesh_neighbor.retain(|&c| c != coord);
        self.remesh_neighbor_pending.remove(&coord);
        self.remesh_priority.retain(|&c| c != coord);
        self.remesh_priority.push_back(coord);
        self.remesh_priority_pending.insert(coord);
    }

    /// Border updates when a neighbor appears — spread out to cap frame-time spikes.
    fn enqueue_neighbor_remesh(&mut self, coord: (i32, i32)) {
        if self.remesh_neighbor_pending.insert(coord) {
            self.remesh_neighbor.push_back(coord);
        }
    }

    /// Drop completed initial loads from the back first; never discard remesh results (GPU may already
    /// have dropped `remesh_in_flight`, so losing a remesh upload leaves stale chunk-border geometry forever).
    fn trim_pending_ready_queue(&mut self) {
        while self.pending_ready.len() > platform::MAX_PENDING_READY_CHUNKS {
            if !self.pending_ready.drop_last_load() {
                break;
            }
        }
    }

    fn dispatch_remesh_job(&mut self, coord: (i32, i32)) -> bool {
        if self.streamer.is_remesh_in_flight(coord) || !self._world.has_chunk(coord) {
            return false;
        }

        let snapshot = Self::mesh_snapshot(&self._world, coord);
        self.streamer.request_remesh(coord, snapshot);
        self.chunks_meshed += 1;
        true
    }

    fn write_chunk_mesh(
        &mut self,
        coord: (i32, i32),
        section_meshes: Vec<(usize, mesh::MeshData)>,
    ) {
        let mut vertices: Vec<Vertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut section_draws: Vec<SectionDrawRange> = Vec::new();
        let mut min_y = i32::MAX;
        let mut max_y = i32::MIN;

        for (section_index, mesh_data) in section_meshes {
            if mesh_data.is_empty() {
                continue;
            }
            let section_world_y = (section_index as i32 + MIN_SECTION_Y) * SECTION_SIZE as i32;
            min_y = min_y.min(section_world_y);
            max_y = max_y.max(section_world_y + SECTION_SIZE as i32);

            let base_vertex = vertices.len() as u32;
            let first_index = indices.len() as u32;
            vertices.extend_from_slice(mesh_data.vertices());
            for &idx in mesh_data.indices() {
                indices.push(base_vertex + idx);
            }
            section_draws.push(SectionDrawRange {
                first_index,
                index_count: mesh_data.indices().len() as u32,
                section_world_y,
            });
        }

        if section_draws.is_empty() {
            self.chunk_meshes.remove(&coord);
            return;
        }

        let surface_max_y = self
            ._world
            .chunk(coord)
            .and_then(|c| c.max_nonempty_world_y())
            .unwrap_or(max_y - 1);

        let cx = coord.0 as f32 * SECTION_SIZE as f32;
        let cz = coord.1 as f32 * SECTION_SIZE as f32;
        let bounds_min = cgmath::Vector3::new(cx, min_y as f32, cz);
        let bounds_max = cgmath::Vector3::new(cx + SECTION_SIZE as f32, max_y as f32, cz + SECTION_SIZE as f32);

        let v_len = (vertices.len() * std::mem::size_of::<Vertex>()) as u64;
        let i_len = (indices.len() * std::mem::size_of::<u32>()) as u64;
        let v_slice = bytemuck::cast_slice(&vertices);
        let i_slice = bytemuck::cast_slice(&indices);

        let reuse = self.chunk_meshes.remove(&coord);
        if let Some(existing) = reuse {
            if existing.vertex_capacity >= v_len && existing.index_capacity >= i_len {
                self.queue.write_buffer(&existing.vertex_buffer, 0, v_slice);
                self.queue.write_buffer(&existing.index_buffer, 0, i_slice);
                self.chunk_meshes.insert(
                    coord,
                    ChunkGpuMesh {
                        section_draws,
                        bounds_min,
                        bounds_max,
                        surface_max_y,
                        ..existing
                    },
                );
                return;
            }
        }

        let v_cap = Self::round_mesh_buffer_capacity(v_len);
        let i_cap = Self::round_mesh_buffer_capacity(i_len);

        let vertex_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Chunk Vertex Buffer"),
            size: v_cap,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&vertex_buffer, 0, v_slice);

        let index_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Chunk Index Buffer"),
            size: i_cap,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&index_buffer, 0, i_slice);

        self.chunk_meshes.insert(
            coord,
            ChunkGpuMesh {
                vertex_buffer,
                index_buffer,
                section_draws,
                vertex_capacity: v_cap,
                index_capacity: i_cap,
                bounds_min,
                bounds_max,
                surface_max_y,
            },
        );
    }

    fn chunk_within_keep_distance(
        coord: (i32, i32),
        player_chunk_x: i32,
        player_chunk_z: i32,
    ) -> bool {
        let dx = coord.0 - player_chunk_x;
        let dz = coord.1 - player_chunk_z;
        dx.abs() <= platform::load_distance_chunks() + platform::UNLOAD_MARGIN_CHUNKS
            && dz.abs() <= platform::load_distance_chunks() + platform::UNLOAD_MARGIN_CHUNKS
    }

    fn chunk_detail_ring(&self, chunk_coord: (i32, i32)) -> u8 {
        let camera_pos = self.camera.position();
        let center = cgmath::Vector3::new(
            chunk_coord.0 as f32 * SECTION_SIZE as f32 + 8.0,
            camera_pos.y,
            chunk_coord.1 as f32 * SECTION_SIZE as f32 + 8.0,
        );
        let horizontal = cgmath::Vector2::new(center.x - camera_pos.x, center.z - camera_pos.z);
        let chunk_dist = horizontal.magnitude() / SECTION_SIZE as f32;
        let near = self.section_draw_distance_chunks * 0.5;
        let mid = self.section_draw_distance_chunks * 0.8;
        if chunk_dist <= near {
            0
        } else if chunk_dist <= mid {
            1
        } else if chunk_dist <= self.section_draw_distance_chunks {
            2
        } else {
            3
        }
    }

    fn chunk_within_draw_distance(&self, chunk_coord: (i32, i32)) -> bool {
        let camera_pos = self.camera.position();
        let cx = chunk_coord.0 as f32 * SECTION_SIZE as f32 + 8.0;
        let cz = chunk_coord.1 as f32 * SECTION_SIZE as f32 + 8.0;
        let horizontal = cgmath::Vector2::new(cx - camera_pos.x, cz - camera_pos.z);
        let max_distance = SECTION_SIZE as f32 * self.section_draw_distance_chunks;
        horizontal.magnitude2() <= max_distance * max_distance
    }

    fn section_passes_surface_lod(
        ring: u8,
        section_world_y: i32,
        surface_max_y: i32,
    ) -> bool {
        if ring < platform::FAR_DETAIL_RING {
            return true;
        }
        let section_top = section_world_y + SECTION_SIZE as i32;
        section_top >= surface_max_y - platform::SURFACE_LOD_DEPTH_BLOCKS
    }

    fn chunk_visible(&self, frustum: &cull::Frustum, chunk_coord: (i32, i32), mesh: &ChunkGpuMesh) -> bool {
        if !self.chunk_within_draw_distance(chunk_coord) {
            return false;
        }
        let cp = self.camera.position();
        let origin = cgmath::Vector3::new(cp.x, cp.y, cp.z);
        frustum.intersects_aabb(mesh.bounds_min - origin, mesh.bounds_max - origin)
    }

    async fn new(window: Arc<Window>, world_source: Arc<dyn source::WorldSource>) -> Self {
        let (width, height) = initial_surface_size(&window);
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let surface = instance.create_surface(window.clone()).unwrap();

        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = window.set_cursor_grab(winit::window::CursorGrabMode::Locked);
            window.set_cursor_visible(false);
        }
        #[cfg(target_arch = "wasm32")]
        {
            window.set_cursor_visible(true);
        }

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
            width,
            height,
            present_mode,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        let camera = camera::Camera::new(
            width as f32 / height as f32,
            45.0,
            0.1,
            1200.0,
        );

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
                        // Vertex reads sky.camera_pos for the camera-relative transform in
                        // square.wgsl; fragment reads it for fog and sky tint.
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
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
                // Every quad the mesher emits winds counter-clockwise as seen from outside the
                // block it belongs to, which `tests/correctness_winding.rs` pins down by
                // rendering a chunk from both sides. Non-opaque blocks emit a second, reversed
                // copy of each face so they still read correctly from inside.
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState {
                    constant: 1,
                    slope_scale: 1.0,
                    clamp: 0.0,
                },
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let hud = hud::HudOverlay::new(&device, config.format);

        let world = World::new();
        let procedural_world = world_source.is_procedural();
        #[cfg(target_arch = "wasm32")]
        let world_seed = world_source.seed();
        #[cfg(target_arch = "wasm32")]
        let mut streamer = ChunkStreamer::new(world_source);
        #[cfg(not(target_arch = "wasm32"))]
        let streamer = ChunkStreamer::new(world_source);
        #[cfg(target_arch = "wasm32")]
        if crate::platform::use_wasm_chunk_workers() {
            use crate::worker_bridge::WorkerSource;
            // Seed worlds hand workers a seed; zip worlds hand them the archive bytes.
            let worker_source = match world_seed {
                Some(seed) => Some(WorkerSource::Seed(seed)),
                None => None,
            };
            let zip = if worker_source.is_none() {
                web_api::clone_init_zip()
            } else {
                None
            };
            let worker_source =
                worker_source.or_else(|| zip.as_deref().map(WorkerSource::Zip));

            if let Some(worker_source) = worker_source {
                if let Err(err) = streamer.enable_workers(worker_source) {
                    web_sys::console::error_1(&JsValue::from_str(&format!(
                        "Failed to start chunk workers: {err} — using main thread"
                    )));
                } else {
                    web_api::set_load_status("Chunk workers started…");
                }
            }
        }
        let pending_ready = PendingReady::new();
        let chunk_meshes = HashMap::new();
        let remesh_priority = VecDeque::new();
        let remesh_neighbor = VecDeque::new();
        let remesh_priority_pending = HashSet::new();
        let remesh_neighbor_pending = HashSet::new();

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
            pending_ready,
            chunk_meshes,
            remesh_priority,
            remesh_neighbor,
            remesh_priority_pending,
            remesh_neighbor_pending,
            render_pipeline,
            camera,
            camera_uniform,
            sky_uniform,
            camera_buffer,
            sky_buffer,
            camera_bind_group,
            texture_bind_group: textures.bind_group,
            camera_controller,
            mouse_captured: cfg!(not(target_arch = "wasm32")),
            is_surface_configured: false,
            last_frame_time: Instant::now(),
            delta: 0,
            depth_texture: depth_texture_view,
            visible_draw_calls_last_frame: 0,
            visible_chunks_last_frame: 0,
            uploaded_meshes_last_frame: 0,
            profile_accum: 0,
            render_frame_index: 0,
            hud,
            fps_ema: 0.0,
            section_draw_distance_chunks: platform::default_section_draw_distance_chunks(),
            world_ready: false,
            started_at: web_time::Instant::now(),
            bootstrap_ready_cache: HashSet::new(),
            drained_new_last_frame: 0,
            drained_remesh_last_frame: 0,
            phase_stream_us: 0,
            phase_mesh_us: 0,
            bootstrap_mesh_attempted: HashSet::new(),
            chunk_requests_issued: 0,
            chunks_meshed: 0,
            chunk_visibility: HashMap::new(),
            section_graph: cull::SectionGraph::new(),
            reachable_sections_last_frame: 0,
            procedural_world,
        };
    }

    fn update(&mut self) {
        self.update_streaming();
        let draw_distance_blocks = self.section_draw_distance_chunks * SECTION_SIZE as f32;
        // Tight far plane improves depth precision; fog hides the cutoff before the hard cull radius.
        self.camera.set_far_plane((draw_distance_blocks * 1.08).max(400.0));
        let inst_fps = if self.delta > 0 {
            1_000_000.0 / self.delta as f32
        } else {
            self.fps_ema
        };
        if self.fps_ema <= 0.0 {
            self.fps_ema = inst_fps;
        } else {
            self.fps_ema = self.fps_ema * 0.92 + inst_fps * 0.08;
        }
        self.camera_uniform.update(&self.camera);
        self.queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::cast_slice(&[self.camera_uniform]),
        );
        let cp = self.camera.position();
        self.sky_uniform.camera_pos = [cp.x, cp.y, cp.z, 0.0];
        self.sky_uniform.fog_params = [
            draw_distance_blocks * 0.42,
            draw_distance_blocks * 0.94,
            1.0,
            0.0,
        ];
        self.queue
            .write_buffer(&self.sky_buffer, 0, bytemuck::cast_slice(&[self.sky_uniform]));
        self.profile_accum += self.delta;
        if self.profile_accum > 1_000_000 {
            println!(
                "profile t={:.1}s ready={} fps={:.0} visible_draws={} visible_chunks={} uploaded_meshes={} (new={} remesh={}) stream={}us mesh={}us loaded_chunks={} pending={} requests={} meshed={} quads={} reachable_sections={}",
                self.started_at.elapsed().as_secs_f64(),
                self.world_ready,
                self.fps_ema,
                self.visible_draw_calls_last_frame,
                self.visible_chunks_last_frame,
                self.uploaded_meshes_last_frame,
                self.drained_new_last_frame,
                self.drained_remesh_last_frame,
                self.phase_stream_us,
                self.phase_mesh_us,
                self._world.chunks().count(),
                self.pending_ready.len(),
                self.chunk_requests_issued,
                self.chunks_meshed,
                self.resident_quads(),
                self.reachable_sections_last_frame,
            );
            self.profile_accum = 0;
        }
    }

    fn chunk_has_any_gpu_section(&self, coord: (i32, i32)) -> bool {
        self.chunk_meshes.contains_key(&coord)
    }

    /// Quads resident in chunk GPU buffers. Summed on demand — once a second for the profile
    /// line is cheap, and it cannot drift out of sync the way an incremental counter would.
    ///
    /// Worth watching after any change to *when* chunks are meshed: a border meshed against a
    /// neighbour that had not loaded yet emits faces that are buried inside the terrain, so
    /// they never appear on screen and only show up as a higher number here.
    fn resident_quads(&self) -> u64 {
        self.chunk_meshes
            .values()
            .flat_map(|mesh| mesh.section_draws.iter())
            .map(|draw| (draw.index_count / 6) as u64)
            .sum()
    }

    fn chunk_needs_gpu_mesh(&self, coord: (i32, i32)) -> bool {
        self._world
            .chunk(coord)
            .is_some_and(|ch| ch.needs_surface_mesh())
    }

    /// Spawn neighborhood tile: data present, optional GPU mesh if non-empty, and no in-flight mesh work.
    fn chunk_bootstrap_tile_ready(&self, coord: (i32, i32)) -> bool {
        if !self._world.has_chunk(coord) {
            return false;
        }
        if self.world_ready {
            if self.streamer.is_in_flight(coord) || self.streamer.is_remesh_in_flight(coord) {
                return false;
            }
            if self.remesh_priority_pending.contains(&coord)
                || self.remesh_neighbor_pending.contains(&coord)
            {
                return false;
            }
            if self.pending_ready.contains_any(coord) {
                return false;
            }
        }
        if self.chunk_needs_gpu_mesh(coord) && !self.chunk_has_any_gpu_section(coord) {
            return self.bootstrap_mesh_attempted.contains(&coord);
        }
        true
    }

    /// A loading-radius tile is meshed only once every neighbour that will arrive during
    /// bootstrap has arrived, so its borders are culled against real blocks instead of the air
    /// of a chunk that simply has not loaded yet.
    ///
    /// This matters more than it looks. Neighbour remeshing is gated on `world_ready`, so it
    /// never runs during bootstrap, and afterwards it only fires when a *new* chunk lands next
    /// to an existing one — which never happens for the interior of the loading radius, since
    /// those neighbours all loaded long ago. A border meshed against air here stays wrong for
    /// the lifetime of the world, showing up as a grid of walls between the chunks nearest the
    /// player while chunks streamed in later look correct.
    ///
    /// Neighbours *outside* the loading radius are not requested until the world is revealed,
    /// so waiting on them would stall the gate forever. Those borders are the ones the ordinary
    /// neighbour remesh does fix, once streaming starts.
    fn bootstrap_neighbours_loaded(
        &self,
        coord: (i32, i32),
        player_chunk_x: i32,
        player_chunk_z: i32,
    ) -> bool {
        let radius = platform::bootstrap_chunk_radius();
        [(1, 0), (-1, 0), (0, 1), (0, -1)]
            .into_iter()
            .all(|(dx, dz)| {
                let neighbour = (coord.0 + dx, coord.1 + dz);
                let inside_loading_radius = (neighbour.0 - player_chunk_x).abs() <= radius
                    && (neighbour.1 - player_chunk_z).abs() <= radius;
                !inside_loading_radius || self._world.has_chunk(neighbour)
            })
    }

    fn bootstrap_tile_count() -> usize {
        let w = (2 * platform::bootstrap_chunk_radius() + 1) as usize;
        w * w
    }

    /// How many tiles in the loading radius are finished terrain — memoised.
    ///
    /// `chunk_bootstrap_tile_ready` ends up walking section block arrays via
    /// `needs_surface_mesh`, and the uncached version ran it over every tile in the loading
    /// radius twice a frame (once to test the gate, once for the HUD). At radius 16 that is
    /// 1089 tiles × 2, and it was the main reason loading throughput decayed from ~69 to
    /// ~9 chunks/s as the radius filled.
    ///
    /// Sound because readiness is monotonic while loading: chunks inside the loading radius
    /// are never unloaded (keep distance is larger), and a tile that has data and a mesh
    /// keeps them. Only unready tiles are re-tested.
    fn bootstrap_ready_tiles_cached(&mut self, player_chunk_x: i32, player_chunk_z: i32) -> usize {
        let radius = platform::bootstrap_chunk_radius();
        for dz in -radius..=radius {
            for dx in -radius..=radius {
                let c = (player_chunk_x + dx, player_chunk_z + dz);
                if self.bootstrap_ready_cache.contains(&c) {
                    continue;
                }
                if self.chunk_bootstrap_tile_ready(c) {
                    self.bootstrap_ready_cache.insert(c);
                }
            }
        }
        self.bootstrap_ready_cache.len()
    }

    fn try_finish_world_bootstrap(&mut self) {
        if self.world_ready {
            return;
        }
        let px = (self.camera.position().x.floor() as i32).div_euclid(SECTION_SIZE as i32);
        let pz = (self.camera.position().z.floor() as i32).div_euclid(SECTION_SIZE as i32);
        if self.bootstrap_ready_tiles_cached(px, pz) == Self::bootstrap_tile_count() {
            self.world_ready = true;
            self.bootstrap_ready_cache.clear();
            println!(
                "world revealed after {:.1}s ({} chunks built, loading radius {})",
                self.started_at.elapsed().as_secs_f64(),
                Self::bootstrap_tile_count(),
                platform::bootstrap_chunk_radius(),
            );
        }
    }

    fn update_streaming(&mut self) {
        let stream_phase_start = Instant::now();
        let player_chunk_x = (self.camera.position().x.floor() as i32).div_euclid(SECTION_SIZE as i32);
        let player_chunk_z = (self.camera.position().z.floor() as i32).div_euclid(SECTION_SIZE as i32);
        let bootstrap = !self.world_ready;
        let frame_over = if bootstrap {
            0
        } else {
            self.delta.saturating_sub(platform::TARGET_FRAME_TIME_US)
        };
        let forward = self.camera.forward();

        let stream_radius = if bootstrap {
            platform::bootstrap_chunk_radius()
        } else {
            platform::load_distance_chunks()
        };
        let mut desired = Vec::new();
        for dz in -stream_radius..=stream_radius {
            for dx in -stream_radius..=stream_radius {
                let cx = player_chunk_x + dx;
                let cz = player_chunk_z + dz;
                let dist2 = dx * dx + dz * dz;
                let to_chunk = cgmath::Vector3::new(dx as f32, 0.0, dz as f32);
                let facing_score = if dist2 == 0 {
                    1.0
                } else {
                    to_chunk.normalize().dot(forward)
                };
                desired.push(((cx, cz), dist2, facing_score));
            }
        }
        desired.sort_by(|a, b| {
            a.1.cmp(&b.1)
                .then_with(|| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal))
        });

        let mut request_budget = if bootstrap {
            platform::bootstrap_max_new_requests_per_frame()
        } else if self.procedural_world {
            platform::procedural_max_new_requests_per_frame()
        } else {
            platform::max_new_requests_per_frame()
        };
        if bootstrap {
            for dz in -platform::bootstrap_chunk_radius()..=platform::bootstrap_chunk_radius() {
                for dx in -platform::bootstrap_chunk_radius()..=platform::bootstrap_chunk_radius() {
                    let coord = (player_chunk_x + dx, player_chunk_z + dz);
                    if !self._world.has_chunk(coord) && self.streamer.is_worker_load_busy(coord) {
                        self.streamer.release_stale_worker_load(coord);
                    }
                }
            }
        }
        if self.pending_ready.load_backlog() >= platform::MAX_PENDING_LOAD_BACKLOG {
            // Enough finished chunks are already queued to keep the pool busy. Requesting more
            // only pushes the queue into `trim_pending_ready_queue`, which discards completed
            // loads that then have to be generated all over again.
            request_budget = 0;
        }
        for (coord, _, _) in desired.iter().copied() {
            if request_budget == 0 {
                break;
            }
            // A chunk already sitting in `pending_ready` has been generated but not applied,
            // so it is in neither the world nor the in-flight set. Without that third test it
            // is re-requested every frame until the drain loop reaches it.
            if !self._world.has_chunk(coord)
                && !self.pending_ready.contains_load(coord)
                && !self.streamer.is_in_flight(coord)
            {
                if self.streamer.request_chunk(coord, bootstrap) {
                    self.chunk_requests_issued += 1;
                    request_budget -= 1;
                    if request_budget == 0 {
                        break;
                    }
                }
            }
        }

        self.pending_ready
            .retain(|meshed| Self::chunk_within_keep_distance(meshed.coord, player_chunk_x, player_chunk_z));

        let loaded: Vec<(i32, i32)> = self._world.chunks().map(|c| c.coord()).collect();
        for coord in loaded {
            if !Self::chunk_within_keep_distance(coord, player_chunk_x, player_chunk_z) {
                self._world.remove_chunk(coord);
                self.chunk_meshes.remove(&coord);
                self.chunk_visibility.remove(&coord);
                self.pending_ready.retain(|meshed| meshed.coord != coord);
                self.remesh_priority.retain(|&c| c != coord);
                self.remesh_neighbor.retain(|&c| c != coord);
                self.remesh_priority_pending.remove(&coord);
                self.remesh_neighbor_pending.remove(&coord);
                self.streamer.clear_pending_remesh_snapshot(coord);
            }
        }

        let drain_budget = if bootstrap {
            platform::bootstrap_ready_drain_budget_per_frame()
        } else {
            platform::READY_DRAIN_BUDGET_PER_FRAME
        };
        let ready = self.streamer.poll_ready(drain_budget);
        for meshed in ready {
            if Self::chunk_within_keep_distance(meshed.coord, player_chunk_x, player_chunk_z) {
                if meshed.is_remesh {
                    self.pending_ready.push_front(meshed);
                } else {
                    self.pending_ready.push_back(meshed);
                }
            }
        }
        self.trim_pending_ready_queue();

        let (upload_budget_dynamic, upload_time_budget_dynamic) = if bootstrap {
            (
                platform::bootstrap_max_chunk_uploads_per_frame(),
                u128::MAX,
            )
        } else if self.procedural_world {
            (
                platform::procedural_max_chunk_uploads_per_frame(),
                platform::max_upload_time_budget_ms(),
            )
        } else if cfg!(target_arch = "wasm32") {
            (
                platform::max_chunk_uploads_per_frame(),
                platform::max_upload_time_budget_ms(),
            )
        } else if frame_over > 10_000 {
            (2, 1)
        } else if frame_over > 5_000 {
            (3, platform::max_upload_time_budget_ms())
        } else {
            (
                platform::max_chunk_uploads_per_frame(),
                platform::max_upload_time_budget_ms(),
            )
        };

        self.phase_stream_us = stream_phase_start.elapsed().as_micros();
        let upload_start = Instant::now();
        let mut uploaded_this_frame = 0usize;
        let mut drained_new = 0usize;
        let mesh_phase_start = Instant::now();
        while uploaded_this_frame < upload_budget_dynamic
            && upload_start.elapsed().as_millis() < upload_time_budget_dynamic
        {
            let Some(meshed) = self.pending_ready.pop_front() else {
                break;
            };
            let coord = meshed.coord;
            if !Self::chunk_within_keep_distance(coord, player_chunk_x, player_chunk_z) {
                continue;
            }
            if let Some(chunk) = meshed.chunk {
                self._world.insert_chunk(chunk);
            }
            if let Some(visibility) = meshed.visibility {
                self.chunk_visibility.insert(coord, visibility);
            }
            let had_initial_meshes = !meshed.is_remesh && !meshed.section_meshes.is_empty();
            if meshed.is_remesh {
                self.upload_chunk_meshes(coord, meshed.section_meshes);
                if bootstrap {
                    // The mesh job dispatched by the bootstrap scan has landed. Record it here
                    // rather than at dispatch, so the loading gate does not count a tile whose
                    // mesh is still being built.
                    self.bootstrap_mesh_attempted.insert(coord);
                }
            } else if bootstrap {
                // Meshing is dispatched by the bootstrap scan below, on the worker pool.
            } else if had_initial_meshes {
                self.upload_chunk_meshes(coord, meshed.section_meshes);
            } else {
                self.enqueue_priority_remesh(coord);
            }
            uploaded_this_frame += 1;
            if !meshed.is_remesh {
                drained_new += 1;
            }

            if !meshed.is_remesh && self.world_ready {
                let neighbors = [
                    (coord.0 + 1, coord.1),
                    (coord.0 - 1, coord.1),
                    (coord.0, coord.1 + 1),
                    (coord.0, coord.1 - 1),
                ];
                for target in neighbors {
                    if self._world.has_chunk(target) {
                        self.enqueue_neighbor_remesh(target);
                    }
                }
            }
        }
        self.phase_mesh_us = mesh_phase_start.elapsed().as_micros();
        self.uploaded_meshes_last_frame = uploaded_this_frame;
        self.drained_new_last_frame = drained_new;
        self.drained_remesh_last_frame = uploaded_this_frame - drained_new;

        // Bootstrap meshing runs on the worker pool. Meshing is ~4.6 ms/chunk and used to run
        // inline here, which made it ~100% of every loading frame; the pool turns the loading
        // wait into whatever the slower of generation and meshing costs across all cores.
        if bootstrap {
            let mut need_mesh: Vec<((i32, i32), i32)> = Vec::new();
            for dz in -platform::bootstrap_chunk_radius()..=platform::bootstrap_chunk_radius() {
                for dx in -platform::bootstrap_chunk_radius()..=platform::bootstrap_chunk_radius() {
                    let coord = (player_chunk_x + dx, player_chunk_z + dz);
                    // Order matters: `chunk_needs_gpu_mesh` walks section block arrays, and
                    // this loop runs over every tile in the loading radius each frame. With
                    // the expensive test first, cost grew as the radius filled and loading
                    // throughput decayed from ~69 to ~9 chunks/s. Cheap set/map lookups
                    // first, so an already-handled tile never reaches the scan.
                    if !self.bootstrap_mesh_attempted.contains(&coord)
                        && !self.chunk_has_any_gpu_section(coord)
                        && !self.streamer.is_remesh_in_flight(coord)
                        && self._world.has_chunk(coord)
                        && self.bootstrap_neighbours_loaded(coord, player_chunk_x, player_chunk_z)
                        && self.chunk_needs_gpu_mesh(coord)
                    {
                        need_mesh.push((coord, dx * dx + dz * dz));
                    }
                }
            }
            need_mesh.sort_by_key(|(_, dist2)| *dist2);
            for (coord, _) in need_mesh
                .into_iter()
                .take(platform::bootstrap_mesh_dispatch_per_frame())
            {
                self.dispatch_remesh_job(coord);
            }
        }

        let extra_priority = if uploaded_this_frame > 0 { 1 } else { 0 };
        let remesh_slowdown = if frame_over > 10_000 { 1 } else { 0 };
        let backlog = self.pending_ready.len() >= platform::PENDING_BACKLOG_REMESH_THRESHOLD;
        let extra_pri_backlog = if backlog {
            platform::PENDING_BACKLOG_EXTRA_PRI_REMESH
        } else {
            0
        };
        let extra_neighbor_backlog = if backlog {
            platform::PENDING_BACKLOG_EXTRA_NEIGHBOR_REMESH
        } else {
            0
        };
        let extra_pri_bootstrap = if bootstrap {
            platform::bootstrap_extra_pri_remesh_per_frame()
        } else {
            0
        };
        let extra_neighbor_bootstrap = if bootstrap {
            platform::bootstrap_extra_neighbor_remesh_per_frame()
        } else {
            0
        };
        let remesh_time_budget = if self.procedural_world {
            platform::procedural_remesh_time_budget_ms()
        } else {
            platform::remesh_time_budget_ms()
        };
        let remesh_start = Instant::now();
        let base_pri_budget = if self.procedural_world {
            platform::PROCEDURAL_REMESH_PRIORITY_BUDGET_PER_FRAME
        } else {
            platform::REMESH_PRIORITY_BUDGET_PER_FRAME
        };
        // Finished mesh results are never discarded by `trim_pending_ready_queue`, so when they
        // outrun the upload budget the queue grows past `MAX_PENDING_READY_CHUNKS` and the trim
        // starts dropping completed *loads* instead — which are then regenerated from scratch.
        // Stop feeding the mesh pool until the results already in hand have been applied.
        let mesh_backpressure =
            self.pending_ready.len() >= platform::MAX_PENDING_READY_CHUNKS / 2;
        let pri_budget = if bootstrap || mesh_backpressure {
            0
        } else {
            (base_pri_budget
                + extra_priority
                + extra_pri_backlog
                + extra_pri_bootstrap)
                .saturating_sub(remesh_slowdown)
        };
        for _ in 0..pri_budget {
            if remesh_start.elapsed().as_millis() > remesh_time_budget {
                break;
            }
            let Some(coord) = self.remesh_priority.pop_front() else {
                break;
            };
            if self.dispatch_remesh_job(coord) {
                self.remesh_priority_pending.remove(&coord);
            } else {
                // Keep pending and retry later instead of dropping the remesh request.
                self.remesh_priority.push_back(coord);
            }
        }
        let base_neighbor_budget = if self.procedural_world {
            platform::PROCEDURAL_NEIGHBOR_REMESH_BUDGET_PER_FRAME
        } else {
            platform::NEIGHBOR_REMESH_BUDGET_PER_FRAME
        };
        let neighbor_budget = if mesh_backpressure {
            0
        } else {
            (base_neighbor_budget + extra_neighbor_backlog + extra_neighbor_bootstrap)
                .saturating_sub(remesh_slowdown)
        };
        for _ in 0..neighbor_budget {
            if bootstrap {
                break;
            }
            if remesh_start.elapsed().as_millis() > remesh_time_budget {
                break;
            }
            let Some(coord) = self.remesh_neighbor.pop_front() else {
                break;
            };
            if self.dispatch_remesh_job(coord) {
                self.remesh_neighbor_pending.remove(&coord);
            } else {
                // Keep pending and retry later instead of dropping the remesh request.
                self.remesh_neighbor.push_back(coord);
            }
        }

        self.try_finish_world_bootstrap();
    }

    fn upload_chunk_meshes(&mut self, coord: (i32, i32), section_meshes: Vec<(usize, mesh::MeshData)>) {
        self.write_chunk_mesh(coord, section_meshes);
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
        self.visible_draw_calls_last_frame = 0;
        self.visible_chunks_last_frame = 0;

        let view_proj = self.camera.build_view_projection_matrix();
        let frustum = cull::Frustum::from_view_projection(&view_proj);
        let player_chunk_x = (self.camera.position().x.floor() as i32).div_euclid(SECTION_SIZE as i32);
        let player_chunk_z = (self.camera.position().z.floor() as i32).div_euclid(SECTION_SIZE as i32);
        let draw_radius = self.section_draw_distance_chunks.ceil() as i32 + 1;

        // Cave culling: walk outward from the camera's section through the connectivity graph,
        // so only sections an actual sightline reaches are drawn. Two disjoint field borrows —
        // the graph is mutated while the visibility map is read.
        let occlusion_culling = platform::section_occlusion_culling();
        if occlusion_culling {
            let camera_section = ((self.camera.position().y.floor() as i32)
                .div_euclid(SECTION_SIZE as i32)
                - MIN_SECTION_Y)
                .clamp(0, SECTION_COUNT as i32 - 1) as usize;
            let cp = self.camera.position();
            let camera_origin = cgmath::Vector3::new(cp.x, cp.y, cp.z);
            let graph = &mut self.section_graph;
            let visibility_map = &self.chunk_visibility;
            let mut reachable = 0usize;
            graph.walk(
                ((player_chunk_x, player_chunk_z), camera_section),
                draw_radius,
                |key| {
                    visibility_map
                        .get(&key.0)
                        .map(|sections| sections[key.1])
                        .unwrap_or(visibility::VisibilitySet::EMPTY)
                },
                |key| {
                    let base_y = (key.1 as i32 + MIN_SECTION_Y) * SECTION_SIZE as i32;
                    // `build_view_projection_matrix` views from the origin to keep float
                    // precision stable far from spawn, so the frustum's planes are in
                    // camera-relative space and the AABB has to be shifted to match.
                    let min = cgmath::Vector3::new(
                        key.0.0 as f32 * SECTION_SIZE as f32 - camera_origin.x,
                        base_y as f32 - camera_origin.y,
                        key.0.1 as f32 * SECTION_SIZE as f32 - camera_origin.z,
                    );
                    let max = min + cgmath::Vector3::new(
                        SECTION_SIZE as f32,
                        SECTION_SIZE as f32,
                        SECTION_SIZE as f32,
                    );
                    frustum.intersects_aabb(min, max)
                },
                |_| reachable += 1,
            );
            self.reachable_sections_last_frame = reachable;
        }

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

            // Initial generation phase: draw no world geometry until the spawn area is
            // ready. The loading HUD covers the screen and the freed frame time goes to
            // chunk generation/meshing (like Minecraft's "Building terrain" gate).
            // Streaming still runs in update() — only the visual draw is skipped here.
            if self.world_ready {
            for dz in -draw_radius..=draw_radius {
                for dx in -draw_radius..=draw_radius {
                    let chunk_coord = (player_chunk_x + dx, player_chunk_z + dz);
                    let Some(mesh) = self.chunk_meshes.get(&chunk_coord) else {
                        continue;
                    };
                    if !self.chunk_visible(&frustum, chunk_coord, mesh) {
                        continue;
                    }
                    let ring = self.chunk_detail_ring(chunk_coord);
                    render_pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                    render_pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);

                    let mut chunk_drew = false;
                    for draw in &mesh.section_draws {
                        if !Self::section_passes_surface_lod(ring, draw.section_world_y, mesh.surface_max_y) {
                            continue;
                        }
                        if occlusion_culling {
                            let section_index = (draw.section_world_y.div_euclid(SECTION_SIZE as i32)
                                - MIN_SECTION_Y) as usize;
                            if !self.section_graph.reached((chunk_coord, section_index)) {
                                continue;
                            }
                        }
                        render_pass.draw_indexed(
                            draw.first_index..draw.first_index + draw.index_count,
                            0,
                            0..1,
                        );
                        self.visible_draw_calls_last_frame += 1;
                        chunk_drew = true;
                    }
                    if chunk_drew {
                        self.visible_chunks_last_frame += 1;
                    }
                }
            }
            }
        }

        let stream_blocks = platform::load_distance_chunks() * SECTION_SIZE as i32;
        let section_cull_blocks = (SECTION_SIZE as f32 * self.section_draw_distance_chunks) as i32;
        // The gate already recomputed this in update(); reuse it rather than re-walking
        // every tile a second time just to draw the progress bar.
        let boot_n = if self.world_ready {
            Self::bootstrap_tile_count()
        } else {
            self.bootstrap_ready_cache.len()
        };
        let boot_total = Self::bootstrap_tile_count();
        let hud_text = if self.world_ready {
            format!(
                "FPS: {:.0}\nChunks loaded: {}\nVisible chunks: {}\nSection draws: {}\nStream radius: {} chunks ({} blocks)\nSection draw radius: {:.1} chunks ({} blocks)\nMesh upload queue: {}\n[ / ] adjust draw distance",
                self.fps_ema,
                self._world.chunks().count(),
                self.visible_chunks_last_frame,
                self.visible_draw_calls_last_frame,
                platform::load_distance_chunks(),
                stream_blocks,
                self.section_draw_distance_chunks,
                section_cull_blocks,
                self.pending_ready.len(),
            )
        } else {
            #[cfg(target_arch = "wasm32")]
            let worker_line = self
                .streamer
                .worker_status()
                .map(|(ready, total, queued, in_flight)| {
                    format!("\nWorkers: {ready}/{total} · {queued} queued · {in_flight} in flight")
                })
                .unwrap_or_else(|| "\nLoader: main thread".to_string());
            #[cfg(not(target_arch = "wasm32"))]
            let worker_line = String::new();
            let pct = 100.0 * boot_n as f32 / boot_total.max(1) as f32;
            let elapsed = self.started_at.elapsed().as_secs_f32();
            // Once enough is done to extrapolate, show a remaining estimate — a bare
            // percentage on a minute-long wait reads as a hang.
            let eta = if boot_n > 16 && pct > 1.0 {
                format!(" · ~{:.0}s left", (elapsed / pct * 100.0 - elapsed).max(0.0))
            } else {
                String::new()
            };
            let bar_width = 32usize;
            let filled = ((pct / 100.0) * bar_width as f32).round() as usize;
            let bar: String = std::iter::repeat('#')
                .take(filled.min(bar_width))
                .chain(std::iter::repeat('-').take(bar_width.saturating_sub(filled)))
                .collect();
            format!(
                "Building terrain…\n[{}] {:.0}%{}\n{}/{} chunks ready ({}×{} around you)\nData loaded: {} · Mesh queue: {}{}\nRevealing once the whole area is built",
                bar,
                pct,
                eta,
                boot_n,
                boot_total,
                2 * platform::bootstrap_chunk_radius() + 1,
                2 * platform::bootstrap_chunk_radius() + 1,
                self._world.chunks().count(),
                self.pending_ready.len(),
                worker_line,
            )
        };
        self.hud
            .prepare(&self.queue, &hud_text, self.config.width, self.config.height);

        {
            let mut hud_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("hud pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            self.hud.render(&mut hud_pass);
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
            self.hud.resize(&self.queue, width, height);
        }
    }
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    static READY_STATE: RefCell<Option<State>> = RefCell::new(None);
}

#[cfg(target_arch = "wasm32")]
fn take_ready_state() -> Option<State> {
    READY_STATE.with(|slot| slot.borrow_mut().take())
}

#[cfg(target_arch = "wasm32")]
fn set_ready_state(state: State) {
    READY_STATE.with(|slot| {
        *slot.borrow_mut() = Some(state);
    });
}

#[cfg(target_arch = "wasm32")]
fn create_wasm_window(event_loop: &ActiveEventLoop) -> Result<Window, winit::error::OsError> {
    use wasm_bindgen::JsCast;

    web_api::prepare_canvas_for_game();
    let document = web_sys::window()
        .and_then(|window| window.document())
        .expect("document");
    let canvas = document
        .get_element_by_id("voxel-canvas")
        .and_then(|el| el.dyn_into::<web_sys::HtmlCanvasElement>().ok());
    event_loop.create_window(
        WindowAttributes::default()
            .with_title("Voxel Engine")
            .with_canvas(canvas)
            .with_visible(true),
    )
}

fn initial_surface_size(window: &Window) -> (u32, u32) {
    let size = window.inner_size();
    if size.width > 0 && size.height > 0 {
        return (size.width, size.height);
    }
    #[cfg(target_arch = "wasm32")]
    if let Some((width, height)) = web_api::canvas_pixel_size() {
        return (width, height);
    }
    (800, 600)
}

struct App {
    #[cfg(target_arch = "wasm32")]
    window: Option<Arc<Window>>,
    state: Option<State>,
    #[cfg(target_arch = "wasm32")]
    state_loading: bool,
    #[cfg(target_arch = "wasm32")]
    needs_redraw: bool,
}

impl App {
    fn new() -> Self {
        Self {
            #[cfg(target_arch = "wasm32")]
            window: None,
            state: None,
            #[cfg(target_arch = "wasm32")]
            state_loading: false,
            #[cfg(target_arch = "wasm32")]
            needs_redraw: false,
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn try_start_pending_world(&mut self, event_loop: &ActiveEventLoop) {
        if web_api::take_reset_renderer_loading() {
            self.state_loading = false;
        }

        if self.state.is_some() {
            return;
        }

        if let Some(mut state) = take_ready_state() {
            self.state_loading = false;
            web_api::hide_load_overlay();
            let (width, height) = initial_surface_size(&state.window);
            state.resize(width, height);
            self.state = Some(state);
            self.needs_redraw = true;
            web_api::kick_event_loop();
            return;
        }

        if self.state_loading {
            return;
        }

        let Some(world_source) = web_api::take_pending_world() else {
            return;
        };

        let window = if let Some(window) = self.window.clone() {
            window
        } else {
            let window = match create_wasm_window(event_loop) {
                Ok(window) => Arc::new(window),
                Err(err) => {
                    web_api::set_pending_world(world_source);
                    web_api::set_load_error(&format!("Failed to create window: {err}"));
                    web_api::show_startup_menu("Ready.");
                    return;
                }
            };
            self.window = Some(window.clone());
            window
        };

        self.state_loading = true;
        web_api::set_load_status("Starting WebGPU…");
        wasm_bindgen_futures::spawn_local(async move {
            let mut state = State::new(window.clone(), world_source).await;
            let (width, height) = initial_surface_size(&window);
            state.resize(width, height);
            set_ready_state(state);
            web_api::kick_event_loop();
        });
    }
}

impl ApplicationHandler for App {
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        #[cfg(target_arch = "wasm32")]
        {
            self.try_start_pending_world(event_loop);
            if self.state.is_none() {
                web_api::kick_event_loop();
            } else if self.needs_redraw {
                web_api::kick_event_loop();
            }
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(state) = self.state.as_ref() {
            state.window.request_redraw();
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = event_loop;
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        let attributes = WindowAttributes::default()
            .with_title("Voxel Engine")
            .with_visible(false);
        #[cfg(not(target_arch = "wasm32"))]
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .unwrap(),
        );
        #[cfg(not(target_arch = "wasm32"))]
        {
        // `VOXEL_SEED=<seed>` previews a real Minecraft world from its seed via the
        // parity generator; without it, load the bundled save.
        let world_source: Arc<dyn source::WorldSource> = match std::env::var("VOXEL_SEED")
            .ok()
            .and_then(|s| s.trim().parse::<i64>().ok())
        {
            Some(seed) => {
                println!("generating world from seed {seed} (Minecraft-parity generator)");
                Arc::new(source::SeededProceduralSource::new(seed))
            }
            None => Arc::new(AnvilSource::new("saves/Basic_World")),
        };
        let mut state = pollster::block_on(State::new(window.clone(), world_source));
        let window = Arc::clone(&state.window);
        state.window.set_maximized(true);
        let size = state.window.inner_size();
        state.resize(size.width, size.height);
        state.update();
        state.render().unwrap();
        window.set_visible(true);
        self.state = Some(state);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        #[cfg(target_arch = "wasm32")]
        if self.needs_redraw {
            if let Some(state) = self.state.as_ref() {
                state.window.request_redraw();
                self.needs_redraw = false;
            }
        }

        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if let Some(state) = self.state.as_mut() {
                    state.resize(size.width, size.height);
                    state.window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                if let Some(state) = self.state.as_mut() {
                    let current_time = Instant::now();
                    state.delta = current_time
                        .duration_since(state.last_frame_time)
                        .as_micros();
                    state.last_frame_time = current_time;
                    if state.world_ready {
                        state
                            .camera_controller
                            .update(state.delta, &mut state.camera);
                    } else {
                        state
                            .camera_controller
                            .update_look_only(&mut state.camera);
                    }
                    state.update();
                    state.window.request_redraw();
                    match state.render() {
                        Ok(_) => {}
                        Err(_) => {
                            let size = state.window.inner_size();
                            state.resize(size.width, size.height);
                        }
                    }
                }
            }
            WindowEvent::Focused(focused) => {
                if focused && self.state.is_some() {
                    if let Some(state) = self.state.as_ref() {
                        state.window.request_redraw();
                    }
                }
            }
            WindowEvent::MouseInput {
                state: button_state,
                button,
                ..
            } if button_state == ElementState::Pressed && button == MouseButton::Left => {
                #[cfg(target_arch = "wasm32")]
                if let Some(state) = self.state.as_mut() {
                    if !state.mouse_captured {
                        state.set_mouse_capture(true);
                        web_api::hide_click_to_play();
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
                        PhysicalKey::Code(KeyCode::BracketLeft) => {
                            if key_state == ElementState::Pressed {
                                state.section_draw_distance_chunks =
                                    (state.section_draw_distance_chunks - 1.0).clamp(
                                        platform::MIN_SECTION_DRAW_DISTANCE_CHUNKS,
                                        platform::max_section_draw_distance_chunks(),
                                    );
                            }
                        }
                        PhysicalKey::Code(KeyCode::BracketRight) => {
                            if key_state == ElementState::Pressed {
                                state.section_draw_distance_chunks =
                                    (state.section_draw_distance_chunks + 1.0).clamp(
                                        platform::MIN_SECTION_DRAW_DISTANCE_CHUNKS,
                                        platform::max_section_draw_distance_chunks(),
                                    );
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
                    // Pointer lock (web) intermittently emits a single spurious huge
                    // motion event; a real mouse never travels this far in one event.
                    // Dropping the outlier stops the view from snapping mid-spin.
                    const MAX_MOTION_PER_EVENT: f64 = 500.0;
                    if delta.0.abs() > MAX_MOTION_PER_EVENT || delta.1.abs() > MAX_MOTION_PER_EVENT {
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
#[wasm_bindgen(start)]
pub fn wasm_start() {
    // Dedicated workers import the same WASM bundle; they must not start winit.
    if web_sys::window().is_none() {
        return;
    }
    web_api::init_web_logging();
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
