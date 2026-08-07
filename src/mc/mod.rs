//! Procedural overworld generation (1.18+ / Caves & Cliffs density-function lineage).
//! Engineering target: high fidelity vs Minecraft 26.2 Java Edition for a given seed.
//! User-facing copy must not claim this is official Minecraft or a game substitute.
//!
//! This is a *parity* engineering target: algorithms here mirror specific Java classes and
//! are validated against known-answer vectors (and against chunks from a real save — the
//! ground-truth oracle). It is deliberately separate from the engine's existing approximate
//! generator (`crate::terrain` / `crate::noise`), which stays as the fast default until
//! parity generation is complete.
//!
//! Build order (each stage gated by parity tests before the next begins):
//!   1. `rng`          — Xoroshiro128PlusPlus + named-noise seeding.  ✓
//!   2. `perlin`       — ImprovedNoise + PerlinNoise (octaves).       ✓
//!   3. `normal_noise` — NormalNoise (the primitive climate/terrain noise). ✓
//!   4. `noise_params` — the `NoiseData` table + seed→named-noise instantiation. ✓
//!   5. `density` — the scalar density-function interpreter (node vocabulary). ✓
//!   6. `blended_noise` — BlendedNoise / base_3d_noise. ✓
//!   7. `spline` — CubicSpline + TerrainProvider offset/factor/jaggedness. ✓
//!   8. `overworld` — assembles the terrain density (surface within ±1–3 of the oracle). ✓
//!   9. `caves` — underground/entrances/noodle/pillars/spaghetti carving; `final_density`
//!      matches the real world to 97.1% block-level solid-vs-void agreement. ✓
//!  10. `aquifer` — the `NoiseBasedAquifer` port: which `finalDensity <= 0` positions are
//!      air, water or lava, and where a pressure barrier turns them back to stone. ✓
//!  11. `climate` + `biome` — the 6-axis climate space and the `OverworldBiomeBuilder` port
//!      that generates its 7594 boxes (verified exactly against Mojang's datagen dump). ✓
//!  12. `surface` — `SurfaceRules`/`SurfaceSystem`/`SurfaceRuleData.overworld`: what the
//!      stone column is faced with (grass, sand, gravel, snow, terracotta, bedrock). Needs
//!      biomes (37 `isBiome` branches), hence the order. ✓
//!  13. `chunk::generate_chunk` — the end-to-end pipeline, and the `WorldSource` hook. ✓
//!
//! Next: wire the renderer to `mc::chunk`, and chase the residual documented in HANDOFF.md.
//!
//! NOTE on validation: stages 1–3 are pinned to an independent BigInt reference of the
//! same Java algorithms, so the port is faithful to that transcription. The one piece
//! still to confirm against a *real* exported world (the ground-truth oracle) is the
//! top-level `RandomState` chain — exactly which positional factory feeds each named
//! noise — which belongs to stage 4's wiring, not the primitives here.

pub mod aquifer;
pub mod biome;
pub mod blended_noise;
pub mod caves;
pub mod chunk;
pub mod climate;
pub mod decorate;
pub mod density;
pub mod noise_params;
pub mod normal_noise;
pub mod overworld;
pub mod perlin;
pub mod rng;
pub mod spline;
pub mod surface;
pub mod tree;
pub mod wgrandom;
