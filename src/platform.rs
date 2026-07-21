/// Runtime tuning — tighter on wasm for browser memory and single-threaded meshing.

#[inline]
pub const fn load_distance_chunks() -> i32 {
    if cfg!(target_arch = "wasm32") {
        16
    } else {
        40
    }
}

#[inline]
pub const fn default_section_draw_distance_chunks() -> f32 {
    if cfg!(target_arch = "wasm32") {
        16.0
    } else {
        40.0
    }
}

#[inline]
pub const fn max_section_draw_distance_chunks() -> f32 {
    if cfg!(target_arch = "wasm32") {
        28.0
    } else {
        64.0
    }
}

#[inline]
pub const fn bootstrap_chunk_radius() -> i32 {
    if cfg!(target_arch = "wasm32") {
        3
    } else {
        8
    }
}

#[inline]
pub const fn max_new_requests_per_frame() -> usize {
    if cfg!(target_arch = "wasm32") {
        4
    } else {
        6
    }
}

/// Procedural generation is cheap — request more chunks per frame on wasm.
#[inline]
pub const fn procedural_max_new_requests_per_frame() -> usize {
    if cfg!(target_arch = "wasm32") {
        10
    } else {
        12
    }
}

#[inline]
pub const fn max_chunk_uploads_per_frame() -> usize {
    if cfg!(target_arch = "wasm32") {
        6
    } else {
        4
    }
}

#[inline]
pub const fn procedural_max_chunk_uploads_per_frame() -> usize {
    if cfg!(target_arch = "wasm32") {
        12
    } else {
        8
    }
}

#[inline]
pub const fn max_upload_time_budget_ms() -> u128 {
    if cfg!(target_arch = "wasm32") {
        16
    } else {
        1
    }
}

/// Cap synchronous main-thread meshing per frame (wasm remesh is inline).
#[inline]
pub const fn remesh_time_budget_ms() -> u128 {
    if cfg!(target_arch = "wasm32") {
        8
    } else {
        2
    }
}

#[inline]
pub const fn procedural_remesh_time_budget_ms() -> u128 {
    if cfg!(target_arch = "wasm32") {
        12
    } else {
        4
    }
}

#[inline]
pub const fn bootstrap_max_new_requests_per_frame() -> usize {
    if cfg!(target_arch = "wasm32") {
        24
    } else {
        32
    }
}

#[inline]
pub const fn bootstrap_ready_drain_budget_per_frame() -> usize {
    if cfg!(target_arch = "wasm32") {
        64
    } else {
        256
    }
}

#[inline]
pub const fn bootstrap_max_chunk_uploads_per_frame() -> usize {
    if cfg!(target_arch = "wasm32") {
        16
    } else {
        24
    }
}

#[inline]
pub const fn bootstrap_upload_time_budget_ms() -> u128 {
    if cfg!(target_arch = "wasm32") {
        16
    } else {
        12
    }
}

#[inline]
pub const fn bootstrap_extra_pri_remesh_per_frame() -> usize {
    if cfg!(target_arch = "wasm32") {
        3
    } else {
        8
    }
}

#[inline]
pub const fn bootstrap_extra_neighbor_remesh_per_frame() -> usize {
    if cfg!(target_arch = "wasm32") {
        2
    } else {
        6
    }
}

/// Scale with the machine, leaving a core for the main thread. The bridge clamps to 4.
#[cfg(target_arch = "wasm32")]
pub fn wasm_worker_count() -> usize {
    web_sys::window()
        .map(|window| window.navigator().hardware_concurrency() as usize)
        .map(|cores| cores.saturating_sub(1).clamp(1, 4))
        .unwrap_or(2)
}

#[cfg(not(target_arch = "wasm32"))]
pub const fn wasm_worker_count() -> usize {
    2
}

/// Web chunk workers are fixed; load + mesh run off the main thread.
#[inline]
pub const fn use_wasm_chunk_workers() -> bool {
    true
}

/// Per-worker in-flight cap. The worker wasm instance is not re-entrant, but queueing a
/// second job keeps it busy while a reply crosses back to the main thread.
#[inline]
pub const fn max_worker_jobs_in_flight() -> usize {
    2
}

#[inline]
pub const fn max_worker_job_queue() -> usize {
    48
}

/// On wasm, only decode NBT sections near the surface when loading from disk.
#[inline]
pub const fn surface_only_chunk_load() -> bool {
    cfg!(target_arch = "wasm32")
}

pub const UNLOAD_MARGIN_CHUNKS: i32 = 5;
pub const REMESH_PRIORITY_BUDGET_PER_FRAME: usize = 2;
pub const NEIGHBOR_REMESH_BUDGET_PER_FRAME: usize = 1;
pub const PROCEDURAL_REMESH_PRIORITY_BUDGET_PER_FRAME: usize = 6;
pub const PROCEDURAL_NEIGHBOR_REMESH_BUDGET_PER_FRAME: usize = 3;
pub const READY_DRAIN_BUDGET_PER_FRAME: usize = 64;
pub const MAX_PENDING_READY_CHUNKS: usize = 600;
pub const PENDING_BACKLOG_REMESH_THRESHOLD: usize = 24;
pub const PENDING_BACKLOG_EXTRA_PRI_REMESH: usize = 4;
pub const PENDING_BACKLOG_EXTRA_NEIGHBOR_REMESH: usize = 3;
pub const TARGET_FRAME_TIME_US: u128 = 33_333;
pub const MIN_SECTION_DRAW_DISTANCE_CHUNKS: f32 = 4.0;
pub const SURFACE_LOD_DEPTH_BLOCKS: i32 = 48;
pub const FAR_DETAIL_RING: u8 = 2;

/// How far below the highest block in a chunk we still build GPU meshes.
/// Skips caves and deep stone — major win on wasm.
#[inline]
pub const fn surface_mesh_depth_blocks() -> i32 {
    if cfg!(target_arch = "wasm32") {
        24
    } else {
        48
    }
}
