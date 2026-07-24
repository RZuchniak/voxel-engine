//! Cave carving — the underground/entrances/noodle/pillar/spaghetti density functions
//! that `NoiseRouterData.overworld` layers onto `slopedCheese`.
//!
//! In the router, the near-surface and underground density is:
//! ```text
//! caves = rangeChoice(slopedCheese, -1e6, 1.5625,
//!                     min(slopedCheese, 5·entrances),   // dense: only big entrances cut in
//!                     underground(slopedCheese));         // else: full cave system
//! finalDensity = min(postProcess(slide(caves)), NOODLE);
//! ```
//! This module ports `entrances`, `underground`, `noodle`, `pillars`, `spaghetti2D`, the
//! spaghetti-roughness function, and `QuantizedSpaghettiRarity`. Only the selected branch
//! of each `rangeChoice`/`intervalSelect` is evaluated (matching Java's laziness), so this
//! stays cheap. Everything is transcribed from decompiled 26.2 `NoiseRouterData.java`.

use std::sync::Arc;

use super::noise_params::Noise;
use super::normal_noise::NormalNoise;
use super::rng::PositionalFactory;

/// The named noises the cave functions sample. Grouped here to keep `Overworld` tidy.
pub struct Caves {
    spaghetti_roughness: Arc<NormalNoise>,
    spaghetti_roughness_modulator: Arc<NormalNoise>,
    spaghetti_3d_1: Arc<NormalNoise>,
    spaghetti_3d_2: Arc<NormalNoise>,
    spaghetti_3d_rarity: Arc<NormalNoise>,
    spaghetti_3d_thickness: Arc<NormalNoise>,
    spaghetti_2d: Arc<NormalNoise>,
    spaghetti_2d_elevation: Arc<NormalNoise>,
    spaghetti_2d_modulator: Arc<NormalNoise>,
    spaghetti_2d_thickness: Arc<NormalNoise>,
    cave_entrance: Arc<NormalNoise>,
    cave_layer: Arc<NormalNoise>,
    cave_cheese: Arc<NormalNoise>,
    pillar: Arc<NormalNoise>,
    pillar_rareness: Arc<NormalNoise>,
    pillar_thickness: Arc<NormalNoise>,
    noodle: Arc<NormalNoise>,
    noodle_thickness: Arc<NormalNoise>,
    noodle_ridge_a: Arc<NormalNoise>,
    noodle_ridge_b: Arc<NormalNoise>,
}

impl Caves {
    pub fn new(factory: &PositionalFactory) -> Self {
        let n = |noise: Noise| Arc::new(noise.instantiate(factory));
        Self {
            spaghetti_roughness: n(Noise::SpaghettiRoughness),
            spaghetti_roughness_modulator: n(Noise::SpaghettiRoughnessModulator),
            spaghetti_3d_1: n(Noise::Spaghetti3d1),
            spaghetti_3d_2: n(Noise::Spaghetti3d2),
            spaghetti_3d_rarity: n(Noise::Spaghetti3dRarity),
            spaghetti_3d_thickness: n(Noise::Spaghetti3dThickness),
            spaghetti_2d: n(Noise::Spaghetti2d),
            spaghetti_2d_elevation: n(Noise::Spaghetti2dElevation),
            spaghetti_2d_modulator: n(Noise::Spaghetti2dModulator),
            spaghetti_2d_thickness: n(Noise::Spaghetti2dThickness),
            cave_entrance: n(Noise::CaveEntrance),
            cave_layer: n(Noise::CaveLayer),
            cave_cheese: n(Noise::CaveCheese),
            pillar: n(Noise::Pillar),
            pillar_rareness: n(Noise::PillarRareness),
            pillar_thickness: n(Noise::PillarThickness),
            noodle: n(Noise::Noodle),
            noodle_thickness: n(Noise::NoodleThickness),
            noodle_ridge_a: n(Noise::NoodleRidgeA),
            noodle_ridge_b: n(Noise::NoodleRidgeB),
        }
    }

    /// `SPAGHETTI_ROUGHNESS_FUNCTION` = `roughnessModulator · (|roughnessNoise| − 0.4)`.
    fn spaghetti_roughness(&self, x: f64, y: f64, z: f64) -> f64 {
        let roughness = noise3(&self.spaghetti_roughness, x, y, z, 1.0, 1.0);
        let modulator = mapped(&self.spaghetti_roughness_modulator, x, y, z, 1.0, 1.0, 0.0, -0.1);
        modulator * (roughness.abs() - 0.4)
    }

    /// `entrances` = `min(bigEntrances, roughness + spaghetti3D)`.
    fn entrances(&self, x: f64, y: f64, z: f64) -> f64 {
        let rarity_mod = noise3(&self.spaghetti_3d_rarity, x, y, z, 2.0, 1.0);
        let thickness_mod = mapped(&self.spaghetti_3d_thickness, x, y, z, 1.0, 1.0, -0.065, -0.088);
        let cave1 = wrap_rarity_3d(rarity_mod, &self.spaghetti_3d_1, x, y, z);
        let cave2 = wrap_rarity_3d(rarity_mod, &self.spaghetti_3d_2, x, y, z);
        let spaghetti_3d = (cave1.max(cave2) + thickness_mod).clamp(-1.0, 1.0);

        let big_src = noise3(&self.cave_entrance, x, y, z, 0.75, 0.5);
        let big_entrances = (big_src + 0.37) + clamped_map(y, -10.0, 30.0, 0.3, 0.0);

        big_entrances.min(self.spaghetti_roughness(x, y, z) + spaghetti_3d)
    }

    /// `spaghetti2D` = `clamp(max(caveNoise, layerRidged), -1, 1)`.
    fn spaghetti_2d(&self, x: f64, y: f64, z: f64) -> f64 {
        let rarity_mod = noise3(&self.spaghetti_2d_modulator, x, y, z, 2.0, 1.0);
        let cave = wrap_rarity_2d(rarity_mod, &self.spaghetti_2d, x, y, z);
        // elevation modulator: mappedNoise(SPAGHETTI_2D_ELEVATION, yScale=0, min=-8, max=8).
        let elevation = mapped(&self.spaghetti_2d_elevation, x, y, z, 1.0, 0.0, -8.0, 8.0);
        let thickness = self.spaghetti_2d_thickness(x, y, z);
        let sloped = (elevation + clamped_map(y, -64.0, 320.0, 8.0, -40.0)).abs();
        let layer_ridged = cube(sloped + thickness);
        let cave_noise = cave + 0.083 * thickness;
        cave_noise.max(layer_ridged).clamp(-1.0, 1.0)
    }

    /// `SPAGHETTI_2D_THICKNESS_MODULATOR` = `mappedNoise(…, xz=2, y=1, min=-0.6, max=-1.3)`.
    fn spaghetti_2d_thickness(&self, x: f64, y: f64, z: f64) -> f64 {
        mapped(&self.spaghetti_2d_thickness, x, y, z, 2.0, 1.0, -0.6, -1.3)
    }

    /// `pillars` = `pillarsWithRareness · pillarThickness³`.
    fn pillars(&self, x: f64, y: f64, z: f64) -> f64 {
        let pillar = noise3(&self.pillar, x, y, z, 25.0, 0.3);
        let rareness = mapped(&self.pillar_rareness, x, y, z, 1.0, 1.0, 0.0, -2.0);
        let thickness = mapped(&self.pillar_thickness, x, y, z, 1.0, 1.0, 0.0, 1.1);
        let with_rareness = pillar * 2.0 + rareness;
        with_rareness * cube(thickness)
    }

    /// `underground(slopedCheese)` = `max(undergroundSubtractions, pillars-with-cutoff)`.
    fn underground(&self, x: f64, y: f64, z: f64, sloped_cheese: f64) -> f64 {
        let layer_src = noise3(&self.cave_layer, x, y, z, 1.0, 8.0);
        let layerized = 4.0 * (layer_src * layer_src);
        let cheese = noise3(&self.cave_cheese, x, y, z, 1.0, 0.666_666_666_666_666_6);
        let solidified = (0.27 + cheese).clamp(-1.0, 1.0)
            + (1.5 + -0.64 * sloped_cheese).clamp(0.0, 0.5);
        let base_cave = layerized + solidified;

        let underground_subtractions = base_cave
            .min(self.entrances(x, y, z))
            .min(self.spaghetti_2d(x, y, z) + self.spaghetti_roughness(x, y, z));

        // pillars = rangeChoice(pillars, -1e6, 0.03, -1e6, pillars): below 0.03 → removed.
        let pillars = self.pillars(x, y, z);
        let pillars_cut = if pillars < 0.03 { -1_000_000.0 } else { pillars };

        underground_subtractions.max(pillars_cut)
    }

    /// `NOODLE` — the thin worm-caves min'd onto the final density.
    fn noodle(&self, x: f64, y: f64, z: f64) -> f64 {
        // yLimitedInterpolatable(y, whenInRange, -60, 320, out) = whenInRange for -60<=y<=320.
        let in_range = (-60.0..=320.0).contains(&y);
        let toggle = if in_range { noise3(&self.noodle, x, y, z, 1.0, 1.0) } else { -1.0 };
        if toggle < 0.0 {
            // rangeChoice(toggle, -1e6, 0.0, 64.0, …): toggle < 0 → solid filler 64.
            return 64.0;
        }
        let thickness = if in_range {
            mapped(&self.noodle_thickness, x, y, z, 1.0, 1.0, -0.05, -0.1)
        } else {
            0.0
        };
        let ridge_a = if in_range { noise3(&self.noodle_ridge_a, x, y, z, 2.666_666_666_666_666_5, 2.666_666_666_666_666_5) } else { 0.0 };
        let ridge_b = if in_range { noise3(&self.noodle_ridge_b, x, y, z, 2.666_666_666_666_666_5, 2.666_666_666_666_666_5) } else { 0.0 };
        let ridged = 1.5 * ridge_a.abs().max(ridge_b.abs());
        thickness + ridged
    }

    /// `caves` = the near-surface/underground selection over `slopedCheese`.
    pub fn caves(&self, x: f64, y: f64, z: f64, sloped_cheese: f64) -> f64 {
        // rangeChoice(slopedCheese, -1e6, 1.5625, min(slopedCheese, 5·entrances), underground).
        if (-1_000_000.0..1.5625).contains(&sloped_cheese) {
            sloped_cheese.min(5.0 * self.entrances(x, y, z))
        } else {
            self.underground(x, y, z, sloped_cheese)
        }
    }

    /// `min(postProcessedSlide, NOODLE)` — apply the NOODLE min after the slide/squeeze.
    pub fn apply_noodle(&self, x: f64, y: f64, z: f64, post_processed: f64) -> f64 {
        post_processed.min(self.noodle(x, y, z))
    }
}

// -------- QuantizedSpaghettiRarity: intervalSelect over the rarity modulator --------

/// `noiseFunctionForRarity(noise, rarity)` = `rarity · noise.getValue(x/rarity, y/rarity, z/rarity)`.
#[inline]
fn noise_for_rarity(noise: &NormalNoise, rarity: f64, x: f64, y: f64, z: f64) -> f64 {
    rarity * noise.get_value(x / rarity, y / rarity, z / rarity)
}

/// `wrapRarity3d` — thresholds [-0.5, 0, 0.5] → rarities [0.75, 1.0, 1.5, 2.0], then `abs`.
fn wrap_rarity_3d(input: f64, noise: &NormalNoise, x: f64, y: f64, z: f64) -> f64 {
    let rarity = if input < -0.5 {
        0.75
    } else if input < 0.0 {
        1.0
    } else if input < 0.5 {
        1.5
    } else {
        2.0
    };
    noise_for_rarity(noise, rarity, x, y, z).abs()
}

/// `wrapRarity2d` — thresholds [-0.75, -0.5, 0.5, 0.75] → rarities [0.5, 0.75, 1.0, 2.0, 3.0], then `abs`.
fn wrap_rarity_2d(input: f64, noise: &NormalNoise, x: f64, y: f64, z: f64) -> f64 {
    let rarity = if input < -0.75 {
        0.5
    } else if input < -0.5 {
        0.75
    } else if input < 0.5 {
        1.0
    } else if input < 0.75 {
        2.0
    } else {
        3.0
    };
    noise_for_rarity(noise, rarity, x, y, z).abs()
}

// -------- small node helpers (shared shapes) --------

/// `noise(noiseData, xzScale, yScale)` at a block position.
#[inline]
fn noise3(n: &NormalNoise, x: f64, y: f64, z: f64, xz_scale: f64, y_scale: f64) -> f64 {
    n.get_value(x * xz_scale, y * y_scale, z * xz_scale)
}

/// `mappedNoise(noiseData, xzScale, yScale, min, max)` = remap unit noise to `[min, max]`.
#[inline]
fn mapped(n: &NormalNoise, x: f64, y: f64, z: f64, xz_scale: f64, y_scale: f64, min: f64, max: f64) -> f64 {
    let middle = (min + max) * 0.5;
    let factor = (max - min) * 0.5;
    middle + factor * noise3(n, x, y, z, xz_scale, y_scale)
}

#[inline]
fn cube(v: f64) -> f64 {
    v * v * v
}

/// `Mth.clampedMap` (double) — reused shape from the terrain assembly.
#[inline]
fn clamped_map(value: f64, from_min: f64, from_max: f64, to_min: f64, to_max: f64) -> f64 {
    let factor = (value - from_min) / (from_max - from_min);
    if factor < 0.0 {
        to_min
    } else if factor > 1.0 {
        to_max
    } else {
        to_min + factor * (to_max - to_min)
    }
}
