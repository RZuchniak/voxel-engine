//! Voxel engine library. The binary (`src/main.rs`) is a thin shell over this crate;
//! integration tests in `tests/` and the headless perf harness consume it directly.
//!
//! Modules migrate here incrementally — see the plan in
//! `.claude/plans/` for the intended final layout.

pub mod biome_tint;
pub mod block;
pub mod camera;
pub mod cull;
pub mod mc;
pub mod noise;
pub mod mesh;
pub mod platform;
pub mod render;
pub mod source;
pub mod terrain;
pub mod texture;
pub mod visibility;
pub mod world;
// Pure data (serde over Chunk/MeshData) with no wasm dependencies, so it lives in the lib
// and stays testable natively even though only the wasm build uses it at runtime.
pub mod worker_protocol;

// `mesh.rs` refers to `crate::Vertex`; re-export so it compiles unchanged in the lib.
pub use render::{OPENGL_TO_WGPU_MATRIX, Vertex};
