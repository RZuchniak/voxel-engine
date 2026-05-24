use std::collections::HashSet;
use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender, unbounded};
#[cfg(not(target_arch = "wasm32"))]
use rayon::ThreadPool;

use crate::{
    mesh::{mesh_chunk_surface, MeshData},
    source::WorldSource,
    world::{Chunk, World},
};

#[cfg(target_arch = "wasm32")]
use crate::worker_bridge::{take_failures, WorkerBridge};

pub struct MeshedChunk {
    pub coord: (i32, i32),
    pub chunk: Option<Chunk>,
    pub section_meshes: Vec<(usize, MeshData)>,
    pub is_remesh: bool,
}

pub struct ChunkStreamer {
    source: Arc<dyn WorldSource>,
    #[cfg(not(target_arch = "wasm32"))]
    pool: ThreadPool,
    ready_tx: Sender<MeshedChunk>,
    ready_rx: Receiver<MeshedChunk>,
    in_flight: HashSet<(i32, i32)>,
    remesh_in_flight: HashSet<(i32, i32)>,
    pending_remesh_coords: HashSet<(i32, i32)>,
    #[cfg(target_arch = "wasm32")]
    worker_bridge: Option<WorkerBridge>,
}

impl ChunkStreamer {
    pub fn new(source: Arc<dyn WorldSource>) -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        let pool = rayon::ThreadPoolBuilder::new()
            .thread_name(|i| format!("chunk-worker-{i}"))
            .build()
            .expect("failed to create rayon pool");
        let (ready_tx, ready_rx) = unbounded();
        Self {
            source,
            #[cfg(not(target_arch = "wasm32"))]
            pool,
            ready_tx,
            ready_rx,
            in_flight: HashSet::new(),
            remesh_in_flight: HashSet::new(),
            pending_remesh_coords: HashSet::new(),
            #[cfg(target_arch = "wasm32")]
            worker_bridge: None,
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub fn enable_workers(&mut self, zip_bytes: &[u8]) -> Result<(), String> {
        if !crate::platform::use_wasm_chunk_workers() || crate::platform::wasm_worker_count() == 0 {
            return Ok(());
        }
        let bridge = WorkerBridge::start(zip_bytes, crate::platform::wasm_worker_count())?;
        self.worker_bridge = Some(bridge);
        Ok(())
    }

    #[cfg(target_arch = "wasm32")]
    fn spawn_main_thread_load(&self, coord: (i32, i32)) {
        let source = Arc::clone(&self.source);
        let ready_tx = self.ready_tx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let chunk = match source.load_chunk(coord) {
                Ok(chunk) => chunk,
                Err(err) => {
                    web_sys::console::error_1(&wasm_bindgen::JsValue::from_str(&format!(
                        "chunk load failed at ({},{}): {err:#}",
                        coord.0, coord.1
                    )));
                    Chunk::new(coord)
                }
            };
            let _ = ready_tx.send(MeshedChunk {
                coord,
                chunk: Some(chunk),
                section_meshes: Vec::new(),
                is_remesh: false,
            });
            crate::web_api::kick_event_loop();
        });
    }

    fn dispatch_remesh_work(&mut self, coord: (i32, i32), world: World) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let ready_tx = self.ready_tx.clone();
            self.pool.spawn(move || {
                let section_meshes = mesh_chunk_surface(&world, coord);

                let _ = ready_tx.send(MeshedChunk {
                    coord,
                    chunk: None,
                    section_meshes,
                    is_remesh: true,
                });
            });
        }

        #[cfg(target_arch = "wasm32")]
        {
            // Remesh uses the main-thread world snapshot; workers only load chunk data.
            let section_meshes = mesh_chunk_surface(&world, coord);
            let _ = self.ready_tx.send(MeshedChunk {
                coord,
                chunk: None,
                section_meshes,
                is_remesh: true,
            });
        }
    }

    fn on_remesh_result_delivered(&mut self, coord: (i32, i32)) {
        self.remesh_in_flight.remove(&coord);
        if self.pending_remesh_coords.remove(&coord) {
            self.remesh_in_flight.insert(coord);
            self.dispatch_remesh_work(coord, World::new());
        }
    }

    fn apply_worker_failures(&mut self) {
        for failure in take_failures() {
            if failure.is_remesh {
                self.remesh_in_flight.remove(&failure.coord);
            } else if self.worker_bridge.is_some() {
                self.in_flight.insert(failure.coord);
                self.spawn_main_thread_load(failure.coord);
            } else {
                self.in_flight.remove(&failure.coord);
            }
        }
    }

    pub fn request_chunk(&mut self, coord: (i32, i32), bootstrap: bool) -> bool {
        if self.is_in_flight(coord) {
            return false;
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = bootstrap;
            self.in_flight.insert(coord);
            let source = Arc::clone(&self.source);
            let ready_tx = self.ready_tx.clone();
            self.pool.spawn(move || {
                let chunk = match source.load_chunk(coord) {
                    Ok(chunk) => chunk,
                    Err(err) => {
                        eprintln!("chunk load failed at {:?}: {err:#}", coord);
                        Chunk::new(coord)
                    }
                };

                let _ = ready_tx.send(MeshedChunk {
                    coord,
                    chunk: Some(chunk),
                    section_meshes: Vec::new(),
                    is_remesh: false,
                });
            });
            return true;
        }

        #[cfg(target_arch = "wasm32")]
        {
            if !bootstrap {
                if let Some(bridge) = self.worker_bridge.as_mut() {
                    if bridge.try_request_load(coord) {
                        return true;
                    }
                }
            }
            self.in_flight.insert(coord);
            self.spawn_main_thread_load(coord);
            true
        }
    }

    pub fn request_remesh(&mut self, coord: (i32, i32), world: World) {
        if self.is_remesh_in_flight(coord) {
            self.pending_remesh_coords.insert(coord);
            return;
        }
        if self.worker_bridge
            .as_ref()
            .is_some_and(|bridge| bridge.is_load_busy(coord))
        {
            self.pending_remesh_coords.insert(coord);
            return;
        }
        self.remesh_in_flight.insert(coord);
        self.dispatch_remesh_work(coord, world);
    }

    pub fn poll_ready(&mut self, budget: usize) -> Vec<MeshedChunk> {
        self.poll_ready_limited(budget, budget, budget)
    }

    pub fn poll_ready_limited(
        &mut self,
        budget: usize,
        _chunk_load_limit: usize,
        _remesh_section_limit: usize,
    ) -> Vec<MeshedChunk> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = (_chunk_load_limit, _remesh_section_limit);
            let mut out = Vec::new();
            for _ in 0..budget {
                let Ok(chunk) = self.ready_rx.try_recv() else {
                    break;
                };
                if chunk.is_remesh {
                    let coord = chunk.coord;
                    self.on_remesh_result_delivered(coord);
                } else {
                    self.in_flight.remove(&chunk.coord);
                }
                out.push(chunk);
            }
            return out;
        }

        #[cfg(target_arch = "wasm32")]
        {
            self.apply_worker_failures();
            let mut out = Vec::new();
            if let Some(bridge) = self.worker_bridge.as_mut() {
                bridge.poll_into(&mut out, budget);
            }
            while out.len() < budget {
                let Ok(chunk) = self.ready_rx.try_recv() else {
                    break;
                };
                out.push(chunk);
            }
            for chunk in &out {
                if chunk.is_remesh {
                    let coord = chunk.coord;
                    self.on_remesh_result_delivered(coord);
                } else {
                    self.in_flight.remove(&chunk.coord);
                }
            }
            self.apply_worker_failures();
            out
        }
    }

    pub fn is_in_flight(&self, coord: (i32, i32)) -> bool {
        if self.in_flight.contains(&coord) {
            return true;
        }
        #[cfg(target_arch = "wasm32")]
        if self
            .worker_bridge
            .as_ref()
            .is_some_and(|bridge| bridge.is_load_busy(coord))
        {
            return true;
        }
        false
    }

    pub fn is_remesh_in_flight(&self, coord: (i32, i32)) -> bool {
        self.remesh_in_flight.contains(&coord)
    }

    pub fn clear_pending_remesh_snapshot(&mut self, coord: (i32, i32)) {
        self.pending_remesh_coords.remove(&coord);
    }

    pub fn is_worker_load_busy(&self, coord: (i32, i32)) -> bool {
        #[cfg(target_arch = "wasm32")]
        {
            return self
                .worker_bridge
                .as_ref()
                .is_some_and(|bridge| bridge.is_load_busy(coord));
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = coord;
            false
        }
    }

    pub fn release_stale_worker_load(&mut self, coord: (i32, i32)) {
        #[cfg(target_arch = "wasm32")]
        if let Some(bridge) = self.worker_bridge.as_mut() {
            bridge.release_load_coord(coord);
        }
        #[cfg(not(target_arch = "wasm32"))]
        let _ = coord;
    }

    pub fn release_stale_load(&mut self, coord: (i32, i32)) {
        self.in_flight.remove(&coord);
        self.release_stale_worker_load(coord);
    }

    #[cfg(target_arch = "wasm32")]
    pub fn worker_status(&self) -> Option<(usize, usize, usize, usize)> {
        let bridge = self.worker_bridge.as_ref()?;
        Some((
            bridge.workers_ready_count(),
            bridge.worker_count(),
            bridge.queued_job_count(),
            bridge.in_flight_job_count(),
        ))
    }
}
