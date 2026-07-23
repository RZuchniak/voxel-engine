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
//!   1. `rng`          — Xoroshiro128PlusPlus + named-noise seeding.  ← current
//!   2. `perlin`       — ImprovedNoise + PerlinNoise (octaves).       (next)
//!   3. `normal_noise` — NormalNoise (the primitive climate/terrain noise).
//!   4. multi-noise biomes → density functions → surface rules.

pub mod rng;
