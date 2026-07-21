//! Voxel engine library. The binary (`src/main.rs`) is a thin shell over this crate;
//! integration tests in `tests/` and the headless perf harness consume it directly.
//!
//! Modules migrate here incrementally — see the plan in
//! `.claude/plans/` for the intended final layout.

pub mod noise;
