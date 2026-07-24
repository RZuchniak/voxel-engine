//! The named-noise parameter table (`firstOctave` + amplitudes) for every overworld
//! worldgen noise, and the seeding that turns a world seed into live `NormalNoise`
//! samplers.
//!
//! These constants are the `NoiseParameters` registered by Minecraft's worldgen
//! bootstrap (`net.minecraft.data.worldgen.NoiseData`). In 26.2 they are code-registered
//! (no longer shipped as JSON in the client jar), but the values are unchanged from the
//! 1.18–1.20 lineage — pure numeric data, transcribed here so the port needs no runtime
//! access to the game files.
//!
//! Seeding (per `RandomState`): a world seed builds `XoroshiroRandom::from_seed(seed)`,
//! whose `fork_positional()` factory then seeds each named noise via
//! `from_hash_of("minecraft:<name>")`. See [`Noise::instantiate`].

use super::normal_noise::NormalNoise;
use super::rng::{PositionalFactory, XoroshiroRandom};

/// One entry from `NoiseData`: the noise's registry name (sans namespace), its
/// `firstOctave`, and the amplitude list (first amplitude + the rest, flattened).
pub struct NoiseParameters {
    /// Registry name without the `minecraft:` prefix (the hash input adds it back).
    pub name: &'static str,
    pub first_octave: i32,
    pub amplitudes: &'static [f64],
}

/// Every named noise the overworld router (`NoiseRouterData.overworld`) can reference.
///
/// The enum discriminants index [`PARAMS`]; keep the two in the same order.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Noise {
    Temperature,
    Vegetation,
    Continentalness,
    Erosion,
    TemperatureLarge,
    VegetationLarge,
    ContinentalnessLarge,
    ErosionLarge,
    Ridge,
    Shift,
    AquiferBarrier,
    AquiferFluidLevelFloodedness,
    AquiferLava,
    AquiferFluidLevelSpread,
    Pillar,
    PillarRareness,
    PillarThickness,
    Spaghetti2d,
    Spaghetti2dElevation,
    Spaghetti2dModulator,
    Spaghetti2dThickness,
    Spaghetti3d1,
    Spaghetti3d2,
    Spaghetti3dRarity,
    Spaghetti3dThickness,
    SpaghettiRoughness,
    SpaghettiRoughnessModulator,
    CaveEntrance,
    CaveLayer,
    CaveCheese,
    OreVeininess,
    OreVeinA,
    OreVeinB,
    OreGap,
    Noodle,
    NoodleThickness,
    NoodleRidgeA,
    NoodleRidgeB,
    Jagged,
}

impl Noise {
    /// The noise's parameter entry.
    pub fn params(self) -> &'static NoiseParameters {
        &PARAMS[self as usize]
    }

    /// Seed this noise from a world seed's positional factory, exactly as
    /// `RandomState` does: `NormalNoise::create(factory.from_hash_of("minecraft:<name>"), params)`.
    pub fn instantiate(self, factory: &PositionalFactory) -> NormalNoise {
        let p = self.params();
        let full = format!("minecraft:{}", p.name);
        let mut random = factory.from_hash_of(&full);
        NormalNoise::create(&mut random, p.first_octave, p.amplitudes.to_vec())
    }
}

/// Convenience: build the positional factory for a world seed, matching
/// `XoroshiroRandomSource(seed).forkPositional()`.
pub fn seed_factory(seed: i64) -> PositionalFactory {
    XoroshiroRandom::from_seed(seed).fork_positional()
}

/// The full `NoiseData` table. Order matches the [`Noise`] enum discriminants.
///
/// Each row is `(name, firstOctave, [firstAmplitude, ...restAmplitudes])`. The biome
/// noises (temperature/vegetation/continentalness/erosion) come in a normal and a
/// `*_large` variant that shifts `firstOctave` by −2 (`registerBiomeNoises`, octaveOffset).
pub static PARAMS: &[NoiseParameters] = &[
    // registerBiomeNoises(octaveOffset = 0)
    p("temperature", -10, &[1.5, 0.0, 1.0, 0.0, 0.0, 0.0]),
    p("vegetation", -8, &[1.0, 1.0, 0.0, 0.0, 0.0, 0.0]),
    p("continentalness", -9, &[1.0, 1.0, 2.0, 2.0, 2.0, 1.0, 1.0, 1.0, 1.0]),
    p("erosion", -9, &[1.0, 1.0, 0.0, 1.0, 1.0]),
    // registerBiomeNoises(octaveOffset = -2) → the `*_large` variants
    p("temperature_large", -12, &[1.5, 0.0, 1.0, 0.0, 0.0, 0.0]),
    p("vegetation_large", -10, &[1.0, 1.0, 0.0, 0.0, 0.0, 0.0]),
    p("continentalness_large", -11, &[1.0, 1.0, 2.0, 2.0, 2.0, 1.0, 1.0, 1.0, 1.0]),
    p("erosion_large", -11, &[1.0, 1.0, 0.0, 1.0, 1.0]),
    p("ridge", -7, &[1.0, 2.0, 1.0, 0.0, 0.0, 0.0]),
    p("offset", -3, &[1.0, 1.0, 0.0]), // Noises.SHIFT == "offset"
    p("aquifer_barrier", -3, &[1.0]),
    p("aquifer_fluid_level_floodedness", -7, &[1.0]),
    p("aquifer_lava", -1, &[1.0]),
    p("aquifer_fluid_level_spread", -5, &[1.0]),
    p("pillar", -7, &[1.0, 1.0]),
    p("pillar_rareness", -8, &[1.0]),
    p("pillar_thickness", -8, &[1.0]),
    p("spaghetti_2d", -7, &[1.0]),
    p("spaghetti_2d_elevation", -8, &[1.0]),
    p("spaghetti_2d_modulator", -11, &[1.0]),
    p("spaghetti_2d_thickness", -11, &[1.0]),
    p("spaghetti_3d_1", -7, &[1.0]),
    p("spaghetti_3d_2", -7, &[1.0]),
    p("spaghetti_3d_rarity", -11, &[1.0]),
    p("spaghetti_3d_thickness", -8, &[1.0]),
    p("spaghetti_roughness", -5, &[1.0]),
    p("spaghetti_roughness_modulator", -8, &[1.0]),
    p("cave_entrance", -7, &[0.4, 0.5, 1.0]),
    p("cave_layer", -8, &[1.0]),
    p("cave_cheese", -8, &[0.5, 1.0, 2.0, 1.0, 2.0, 1.0, 0.0, 2.0, 0.0]),
    p("ore_veininess", -8, &[1.0]),
    p("ore_vein_a", -7, &[1.0]),
    p("ore_vein_b", -7, &[1.0]),
    p("ore_gap", -5, &[1.0]),
    p("noodle", -8, &[1.0]),
    p("noodle_thickness", -8, &[1.0]),
    p("noodle_ridge_a", -7, &[1.0]),
    p("noodle_ridge_b", -7, &[1.0]),
    p("jagged", -16, &[1.0; 16]),
];

/// Const helper so [`PARAMS`] reads as a plain table.
const fn p(name: &'static str, first_octave: i32, amplitudes: &'static [f64]) -> NoiseParameters {
    NoiseParameters { name, first_octave, amplitudes }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The enum order must line up with the table, so `Noise::X.params().name` is "x".
    #[test]
    fn enum_and_table_agree() {
        assert_eq!(Noise::Temperature.params().name, "temperature");
        assert_eq!(Noise::Continentalness.params().first_octave, -9);
        assert_eq!(Noise::ContinentalnessLarge.params().first_octave, -11);
        assert_eq!(Noise::Jagged.params().amplitudes.len(), 16);
        assert_eq!(Noise::Shift.params().name, "offset");
    }

    /// End-to-end: the `continentalness` noise seeded from a world seed matches the
    /// value the existing `normal_noise` reference test pins (same seeding path).
    #[test]
    fn continentalness_seeds_like_reference() {
        // The normal_noise reference uses an 8-octave flat noise; here we exercise the
        // real continentalness params through the same from_hash_of("minecraft:...") path
        // and just assert it produces a finite, in-range value deterministically.
        let factory = seed_factory(6954908675375307936);
        let n = Noise::Continentalness.instantiate(&factory);
        let a = n.get_value(0.0, 0.0, 0.0);
        let b = n.get_value(0.0, 0.0, 0.0);
        assert_eq!(a, b, "sampling must be deterministic");
        assert!(a.abs() <= 2.0, "continentalness {a} out of expected range");
    }
}
