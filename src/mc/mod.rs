//! Bit-exact reimplementation of Minecraft 1.18+ (Caves & Cliffs) Java Edition overworld
//! generation, so a real world seed can be previewed in-browser and match the game.
//!
//! This is a *parity* target: every algorithm here mirrors a specific Java class and is
//! validated against known-answer vectors (and, later, against chunks exported from a real
//! world — the ground-truth oracle). It is deliberately separate from the engine's
//! existing approximate generator (`crate::terrain` / `crate::noise`), which stays as the
//! fast default until parity generation is complete.
//!
//! Build order (each stage gated by parity tests before the next begins):
//!   1. `rng`          — Xoroshiro128PlusPlus + named-noise seeding.  ✓
//!   2. `perlin`       — ImprovedNoise + PerlinNoise (octaves).       ✓
//!   3. `normal_noise` — NormalNoise (the primitive climate/terrain noise). ✓
//!   4. `noise_params` — the `NoiseData` table + seed→named-noise instantiation. ✓
//!   5. `density` — the scalar density-function interpreter (node vocabulary). ✓
//!   6. `blended_noise` — BlendedNoise / base_3d_noise. ✓
//!   7. `spline` — CubicSpline + TerrainProvider offset/factor/jaggedness. ✓
//!   8. `overworld` — assembles the pre-cave terrain density; validated against the real
//!      "Voxel" world (predicted surface within ±1–3 of the oracle; the residual is the
//!      not-yet-ported surface rules + aquifers). ✓
//!   9. cave carving (rangeChoice/underground/noodle/entrances) + aquifers + surface rules
//!      → then multi-noise biomes.                                    ← next
//!
//! NOTE on validation: stages 1–3 are pinned to an independent BigInt reference of the
//! same Java algorithms, so the port is faithful to that transcription. The one piece
//! still to confirm against a *real* exported world (the ground-truth oracle) is the
//! top-level `RandomState` chain — exactly which positional factory feeds each named
//! noise — which belongs to stage 4's wiring, not the primitives here.

pub mod blended_noise;
pub mod caves;
pub mod density;
pub mod noise_params;
pub mod normal_noise;
pub mod overworld;
pub mod perlin;
pub mod rng;
pub mod spline;
