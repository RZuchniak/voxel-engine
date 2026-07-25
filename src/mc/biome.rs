//! The overworld biome layout — a port of `OverworldBiomeBuilder`, which *constructs* the
//! climate-space boxes rather than shipping them as data.
//!
//! Minecraft's biome table is not a literal table: it's built by nested loops over 5
//! temperature bands × 5 humidity bands × 13 weirdness slices, with the biome at each cell
//! chosen from a handful of 5×5 lookup grids plus rules like "badlands if hot", "slope if
//! cold", "windswept savanna if dry and weird". Reimplementing the generator (rather than
//! copying its 7594-entry output) keeps Mojang's data out of this repo and gives an exact
//! cross-check: `examples/parity_biomes` diffs this against the real datagen dump, entry for
//! entry, and requires a perfect match.
//!
//! Emission order matters and is preserved: off-coast, then inland (slices in the same
//! order), then underground. [`super::climate::ParameterList::find`] breaks ties by first
//! occurrence, so the order is part of the behaviour.

use super::climate::{Parameter, ParameterPoint};

/// A biome's registry name, without the `minecraft:` prefix.
pub type Biome = &'static str;

/// `Parameter.span(-1, 1)` — "any value on this axis".
fn full_range() -> Parameter {
    Parameter::span(-1.0, 1.0)
}

/// The five temperature bands (`temperatures`).
fn temperatures() -> [Parameter; 5] {
    [
        Parameter::span(-1.0, -0.45),
        Parameter::span(-0.45, -0.15),
        Parameter::span(-0.15, 0.2),
        Parameter::span(0.2, 0.55),
        Parameter::span(0.55, 1.0),
    ]
}

/// The five humidity bands (`humidities`).
fn humidities() -> [Parameter; 5] {
    [
        Parameter::span(-1.0, -0.35),
        Parameter::span(-0.35, -0.1),
        Parameter::span(-0.1, 0.1),
        Parameter::span(0.1, 0.3),
        Parameter::span(0.3, 1.0),
    ]
}

/// The seven erosion bands (`erosions`).
fn erosions() -> [Parameter; 7] {
    [
        Parameter::span(-1.0, -0.78),
        Parameter::span(-0.78, -0.375),
        Parameter::span(-0.375, -0.2225),
        Parameter::span(-0.2225, 0.05),
        Parameter::span(0.05, 0.45),
        Parameter::span(0.45, 0.55),
        Parameter::span(0.55, 1.0),
    ]
}

#[rustfmt::skip]
const OCEANS: [[Biome; 5]; 2] = [
    ["deep_frozen_ocean", "deep_cold_ocean", "deep_ocean", "deep_lukewarm_ocean", "warm_ocean"],
    ["frozen_ocean",      "cold_ocean",      "ocean",      "lukewarm_ocean",      "warm_ocean"],
];

#[rustfmt::skip]
const MIDDLE_BIOMES: [[Biome; 5]; 5] = [
    ["snowy_plains",  "snowy_plains", "snowy_plains", "snowy_taiga",  "taiga"],
    ["plains",        "plains",       "forest",       "taiga",        "old_growth_spruce_taiga"],
    ["flower_forest", "plains",       "forest",       "birch_forest", "dark_forest"],
    ["savanna",       "savanna",      "forest",       "jungle",       "jungle"],
    ["desert",        "desert",       "desert",       "desert",       "desert"],
];

#[rustfmt::skip]
const MIDDLE_BIOMES_VARIANT: [[Option<Biome>; 5]; 5] = [
    [Some("ice_spikes"),       None,                Some("snowy_taiga"), None,                       None],
    [None,                     None,                None,          None,                             Some("old_growth_pine_taiga")],
    [Some("sunflower_plains"), None,                None,          Some("old_growth_birch_forest"),  None],
    [None,                     None,                Some("plains"), Some("sparse_jungle"),           Some("bamboo_jungle")],
    [None,                     None,                None,          None,                             None],
];

#[rustfmt::skip]
const PLATEAU_BIOMES: [[Biome; 5]; 5] = [
    ["snowy_plains",    "snowy_plains",    "snowy_plains", "snowy_taiga",     "snowy_taiga"],
    ["meadow",          "meadow",          "forest",       "taiga",           "old_growth_spruce_taiga"],
    ["meadow",          "meadow",          "meadow",       "meadow",          "pale_garden"],
    ["savanna_plateau", "savanna_plateau", "forest",       "forest",          "jungle"],
    ["badlands",        "badlands",        "badlands",     "wooded_badlands", "wooded_badlands"],
];

#[rustfmt::skip]
const PLATEAU_BIOMES_VARIANT: [[Option<Biome>; 5]; 5] = [
    [Some("ice_spikes"),      None,                 None,             None,                  None],
    [Some("cherry_grove"),    None,                 Some("meadow"),   Some("meadow"),        Some("old_growth_pine_taiga")],
    [Some("cherry_grove"),    Some("cherry_grove"), Some("forest"),   Some("birch_forest"),  None],
    [None,                    None,                 None,             None,                  None],
    [Some("eroded_badlands"), Some("eroded_badlands"), None,          None,                  None],
];

#[rustfmt::skip]
const SHATTERED_BIOMES: [[Option<Biome>; 5]; 5] = [
    [Some("windswept_gravelly_hills"), Some("windswept_gravelly_hills"), Some("windswept_hills"), Some("windswept_forest"), Some("windswept_forest")],
    [Some("windswept_gravelly_hills"), Some("windswept_gravelly_hills"), Some("windswept_hills"), Some("windswept_forest"), Some("windswept_forest")],
    [Some("windswept_hills"),          Some("windswept_hills"),          Some("windswept_hills"), Some("windswept_forest"), Some("windswept_forest")],
    [None, None, None, None, None],
    [None, None, None, None, None],
];

/// Accumulates the emitted boxes, holding the band tables so the `pick*` rules can read them.
struct Builder {
    out: Vec<(ParameterPoint, Biome)>,
    temperatures: [Parameter; 5],
    humidities: [Parameter; 5],
    erosions: [Parameter; 7],
    mushroom_fields: Parameter,
    deep_ocean: Parameter,
    ocean: Parameter,
    coast: Parameter,
    inland: Parameter,
    near_inland: Parameter,
    mid_inland: Parameter,
    far_inland: Parameter,
}

/// `OverworldBiomeBuilder.addBiomes` — the full overworld biome list, in emission order.
pub fn overworld_biomes() -> Vec<(ParameterPoint, Biome)> {
    let mut b = Builder {
        out: Vec::new(),
        temperatures: temperatures(),
        humidities: humidities(),
        erosions: erosions(),
        mushroom_fields: Parameter::span(-1.2, -1.05),
        deep_ocean: Parameter::span(-1.05, -0.455),
        ocean: Parameter::span(-0.455, -0.19),
        coast: Parameter::span(-0.19, -0.11),
        inland: Parameter::span(-0.11, 0.55),
        near_inland: Parameter::span(-0.11, 0.03),
        mid_inland: Parameter::span(0.03, 0.3),
        far_inland: Parameter::span(0.3, 1.0),
    };
    b.add_off_coast_biomes();
    b.add_inland_biomes();
    b.add_underground_biomes();
    b.out
}

impl Builder {
    // ---- emitters ----

    /// `addSurfaceBiome` — note it emits **two** boxes, at depth 0 (surface) and depth 1
    /// (just under it), so the biome extends down into the ground.
    #[allow(clippy::too_many_arguments)]
    fn surface(
        &mut self,
        temperature: Parameter,
        humidity: Parameter,
        continentalness: Parameter,
        erosion: Parameter,
        weirdness: Parameter,
        offset: f32,
        biome: Biome,
    ) {
        for depth in [Parameter::point(0.0), Parameter::point(1.0)] {
            self.out.push((
                ParameterPoint {
                    temperature,
                    humidity,
                    continentalness,
                    erosion,
                    depth,
                    weirdness,
                    offset: super::climate::quantize(offset),
                },
                biome,
            ));
        }
    }

    /// `addUndergroundBiome` — one box spanning the cave depth band.
    #[allow(clippy::too_many_arguments)]
    fn underground(
        &mut self,
        temperature: Parameter,
        humidity: Parameter,
        continentalness: Parameter,
        erosion: Parameter,
        weirdness: Parameter,
        offset: f32,
        biome: Biome,
    ) {
        self.out.push((
            ParameterPoint {
                temperature,
                humidity,
                continentalness,
                erosion,
                depth: Parameter::span(0.2, 0.9),
                weirdness,
                offset: super::climate::quantize(offset),
            },
            biome,
        ));
    }

    /// `addBottomBiome` — pinned to depth 1.1, below everything else.
    #[allow(clippy::too_many_arguments)]
    fn bottom(
        &mut self,
        temperature: Parameter,
        humidity: Parameter,
        continentalness: Parameter,
        erosion: Parameter,
        weirdness: Parameter,
        offset: f32,
        biome: Biome,
    ) {
        self.out.push((
            ParameterPoint {
                temperature,
                humidity,
                continentalness,
                erosion,
                depth: Parameter::point(1.1),
                weirdness,
                offset: super::climate::quantize(offset),
            },
            biome,
        ));
    }

    // ---- the layout ----

    fn add_off_coast_biomes(&mut self) {
        let full = full_range();
        self.surface(full, full, self.mushroom_fields, full, full, 0.0, "mushroom_fields");
        for i in 0..5 {
            let t = self.temperatures[i];
            self.surface(t, full, self.deep_ocean, full, full, 0.0, OCEANS[0][i]);
            self.surface(t, full, self.ocean, full, full, 0.0, OCEANS[1][i]);
        }
    }

    /// The 13 weirdness slices. Their order and boundaries define the whole terrain-to-biome
    /// mapping (valleys at the centre, peaks either side, mid/high slices outward).
    fn add_inland_biomes(&mut self) {
        self.add_mid_slice(Parameter::span(-1.0, -0.93333334));
        self.add_high_slice(Parameter::span(-0.93333334, -0.7666667));
        self.add_peaks(Parameter::span(-0.7666667, -0.56666666));
        self.add_high_slice(Parameter::span(-0.56666666, -0.4));
        self.add_mid_slice(Parameter::span(-0.4, -0.26666668));
        self.add_low_slice(Parameter::span(-0.26666668, -0.05));
        self.add_valleys(Parameter::span(-0.05, 0.05));
        self.add_low_slice(Parameter::span(0.05, 0.26666668));
        self.add_mid_slice(Parameter::span(0.26666668, 0.4));
        self.add_high_slice(Parameter::span(0.4, 0.56666666));
        self.add_peaks(Parameter::span(0.56666666, 0.7666667));
        self.add_high_slice(Parameter::span(0.7666667, 0.93333334));
        self.add_mid_slice(Parameter::span(0.93333334, 1.0));
    }

    fn add_peaks(&mut self, w: Parameter) {
        for ti in 0..5 {
            let t = self.temperatures[ti];
            for hi in 0..5 {
                let h = self.humidities[hi];
                let middle = self.pick_middle(ti, hi, w);
                let middle_or_badlands = self.pick_middle_or_badlands_if_hot(ti, hi, w);
                let middle_or_badlands_or_slope = self.pick_middle_or_badlands_if_hot_or_slope_if_cold(ti, hi, w);
                let plateau = self.pick_plateau(ti, hi, w);
                let shattered = self.pick_shattered(ti, hi, w);
                let shattered_or_savanna = self.maybe_windswept_savanna(ti, hi, w, shattered);
                let peak = self.pick_peak(ti, hi, w);
                let (e, coast, near, mid, far) = self.bands();

                self.surface(t, h, Parameter::hull(coast, far), e[0], w, 0.0, peak);
                self.surface(t, h, Parameter::hull(coast, near), e[1], w, 0.0, middle_or_badlands_or_slope);
                self.surface(t, h, Parameter::hull(mid, far), e[1], w, 0.0, peak);
                self.surface(t, h, Parameter::hull(coast, near), Parameter::hull(e[2], e[3]), w, 0.0, middle);
                self.surface(t, h, Parameter::hull(mid, far), e[2], w, 0.0, plateau);
                self.surface(t, h, mid, e[3], w, 0.0, middle_or_badlands);
                self.surface(t, h, far, e[3], w, 0.0, plateau);
                self.surface(t, h, Parameter::hull(coast, far), e[4], w, 0.0, middle);
                self.surface(t, h, Parameter::hull(coast, near), e[5], w, 0.0, shattered_or_savanna);
                self.surface(t, h, Parameter::hull(mid, far), e[5], w, 0.0, shattered);
                self.surface(t, h, Parameter::hull(coast, far), e[6], w, 0.0, middle);
            }
        }
    }

    fn add_high_slice(&mut self, w: Parameter) {
        for ti in 0..5 {
            let t = self.temperatures[ti];
            for hi in 0..5 {
                let h = self.humidities[hi];
                let middle = self.pick_middle(ti, hi, w);
                let middle_or_badlands = self.pick_middle_or_badlands_if_hot(ti, hi, w);
                let middle_or_badlands_or_slope = self.pick_middle_or_badlands_if_hot_or_slope_if_cold(ti, hi, w);
                let plateau = self.pick_plateau(ti, hi, w);
                let shattered = self.pick_shattered(ti, hi, w);
                let middle_or_savanna = self.maybe_windswept_savanna(ti, hi, w, middle);
                let slope = self.pick_slope(ti, hi, w);
                let peak = self.pick_peak(ti, hi, w);
                let (e, coast, near, mid, far) = self.bands();

                self.surface(t, h, coast, Parameter::hull(e[0], e[1]), w, 0.0, middle);
                self.surface(t, h, near, e[0], w, 0.0, slope);
                self.surface(t, h, Parameter::hull(mid, far), e[0], w, 0.0, peak);
                self.surface(t, h, near, e[1], w, 0.0, middle_or_badlands_or_slope);
                self.surface(t, h, Parameter::hull(mid, far), e[1], w, 0.0, slope);
                self.surface(t, h, Parameter::hull(coast, near), Parameter::hull(e[2], e[3]), w, 0.0, middle);
                self.surface(t, h, Parameter::hull(mid, far), e[2], w, 0.0, plateau);
                self.surface(t, h, mid, e[3], w, 0.0, middle_or_badlands);
                self.surface(t, h, far, e[3], w, 0.0, plateau);
                self.surface(t, h, Parameter::hull(coast, far), e[4], w, 0.0, middle);
                self.surface(t, h, Parameter::hull(coast, near), e[5], w, 0.0, middle_or_savanna);
                self.surface(t, h, Parameter::hull(mid, far), e[5], w, 0.0, shattered);
                self.surface(t, h, Parameter::hull(coast, far), e[6], w, 0.0, middle);
            }
        }
    }

    fn add_mid_slice(&mut self, w: Parameter) {
        let full = full_range();
        let (e, coast, near, mid, far) = self.bands();
        let t = self.temperatures;
        self.surface(full, full, coast, Parameter::hull(e[0], e[2]), w, 0.0, "stony_shore");
        self.surface(Parameter::hull(t[1], t[2]), full, Parameter::hull(near, far), e[6], w, 0.0, "swamp");
        self.surface(Parameter::hull(t[3], t[4]), full, Parameter::hull(near, far), e[6], w, 0.0, "mangrove_swamp");

        for ti in 0..5 {
            let t = self.temperatures[ti];
            for hi in 0..5 {
                let h = self.humidities[hi];
                let middle = self.pick_middle(ti, hi, w);
                let middle_or_badlands = self.pick_middle_or_badlands_if_hot(ti, hi, w);
                let middle_or_badlands_or_slope = self.pick_middle_or_badlands_if_hot_or_slope_if_cold(ti, hi, w);
                let shattered = self.pick_shattered(ti, hi, w);
                let plateau = self.pick_plateau(ti, hi, w);
                let beach = pick_beach(ti);
                let middle_or_savanna = self.maybe_windswept_savanna(ti, hi, w, middle);
                let shattered_coast = self.pick_shattered_coast(ti, hi, w);
                let slope = self.pick_slope(ti, hi, w);

                self.surface(t, h, Parameter::hull(near, far), e[0], w, 0.0, slope);
                self.surface(t, h, Parameter::hull(near, mid), e[1], w, 0.0, middle_or_badlands_or_slope);
                self.surface(t, h, far, e[1], w, 0.0, if ti == 0 { slope } else { plateau });
                self.surface(t, h, near, e[2], w, 0.0, middle);
                self.surface(t, h, mid, e[2], w, 0.0, middle_or_badlands);
                self.surface(t, h, far, e[2], w, 0.0, plateau);
                self.surface(t, h, Parameter::hull(coast, near), e[3], w, 0.0, middle);
                self.surface(t, h, Parameter::hull(mid, far), e[3], w, 0.0, middle_or_badlands);
                if w.max < 0 {
                    self.surface(t, h, coast, e[4], w, 0.0, beach);
                    self.surface(t, h, Parameter::hull(near, far), e[4], w, 0.0, middle);
                } else {
                    self.surface(t, h, Parameter::hull(coast, far), e[4], w, 0.0, middle);
                }
                self.surface(t, h, coast, e[5], w, 0.0, shattered_coast);
                self.surface(t, h, near, e[5], w, 0.0, middle_or_savanna);
                self.surface(t, h, Parameter::hull(mid, far), e[5], w, 0.0, shattered);
                if w.max < 0 {
                    self.surface(t, h, coast, e[6], w, 0.0, beach);
                } else {
                    self.surface(t, h, coast, e[6], w, 0.0, middle);
                }
                if ti == 0 {
                    self.surface(t, h, Parameter::hull(near, far), e[6], w, 0.0, middle);
                }
            }
        }
    }

    fn add_low_slice(&mut self, w: Parameter) {
        let full = full_range();
        let (e, coast, near, mid, far) = self.bands();
        let t = self.temperatures;
        self.surface(full, full, coast, Parameter::hull(e[0], e[2]), w, 0.0, "stony_shore");
        self.surface(Parameter::hull(t[1], t[2]), full, Parameter::hull(near, far), e[6], w, 0.0, "swamp");
        self.surface(Parameter::hull(t[3], t[4]), full, Parameter::hull(near, far), e[6], w, 0.0, "mangrove_swamp");

        for ti in 0..5 {
            let t = self.temperatures[ti];
            for hi in 0..5 {
                let h = self.humidities[hi];
                let middle = self.pick_middle(ti, hi, w);
                let middle_or_badlands = self.pick_middle_or_badlands_if_hot(ti, hi, w);
                let middle_or_badlands_or_slope = self.pick_middle_or_badlands_if_hot_or_slope_if_cold(ti, hi, w);
                let beach = pick_beach(ti);
                let middle_or_savanna = self.maybe_windswept_savanna(ti, hi, w, middle);
                let shattered_coast = self.pick_shattered_coast(ti, hi, w);

                self.surface(t, h, near, Parameter::hull(e[0], e[1]), w, 0.0, middle_or_badlands);
                self.surface(t, h, Parameter::hull(mid, far), Parameter::hull(e[0], e[1]), w, 0.0, middle_or_badlands_or_slope);
                self.surface(t, h, near, Parameter::hull(e[2], e[3]), w, 0.0, middle);
                self.surface(t, h, Parameter::hull(mid, far), Parameter::hull(e[2], e[3]), w, 0.0, middle_or_badlands);
                self.surface(t, h, coast, Parameter::hull(e[3], e[4]), w, 0.0, beach);
                self.surface(t, h, Parameter::hull(near, far), e[4], w, 0.0, middle);
                self.surface(t, h, coast, e[5], w, 0.0, shattered_coast);
                self.surface(t, h, near, e[5], w, 0.0, middle_or_savanna);
                self.surface(t, h, Parameter::hull(mid, far), e[5], w, 0.0, middle);
                self.surface(t, h, coast, e[6], w, 0.0, beach);
                if ti == 0 {
                    self.surface(t, h, Parameter::hull(near, far), e[6], w, 0.0, middle);
                }
            }
        }
    }

    /// The weirdness band around 0 — where rivers live.
    fn add_valleys(&mut self, w: Parameter) {
        let full = full_range();
        let (e, coast, near, _mid, far) = self.bands();
        let frozen = self.temperatures[0];
        let unfrozen = Parameter::hull(self.temperatures[1], self.temperatures[4]);
        let inland = self.inland;
        let t = self.temperatures;

        let cold_shore = if w.max < 0 { "stony_shore" } else { "frozen_river" };
        let warm_shore = if w.max < 0 { "stony_shore" } else { "river" };
        self.surface(frozen, full, coast, Parameter::hull(e[0], e[1]), w, 0.0, cold_shore);
        self.surface(unfrozen, full, coast, Parameter::hull(e[0], e[1]), w, 0.0, warm_shore);
        self.surface(frozen, full, near, Parameter::hull(e[0], e[1]), w, 0.0, "frozen_river");
        self.surface(unfrozen, full, near, Parameter::hull(e[0], e[1]), w, 0.0, "river");
        self.surface(frozen, full, Parameter::hull(coast, far), Parameter::hull(e[2], e[5]), w, 0.0, "frozen_river");
        self.surface(unfrozen, full, Parameter::hull(coast, far), Parameter::hull(e[2], e[5]), w, 0.0, "river");
        self.surface(frozen, full, coast, e[6], w, 0.0, "frozen_river");
        self.surface(unfrozen, full, coast, e[6], w, 0.0, "river");
        self.surface(Parameter::hull(t[1], t[2]), full, Parameter::hull(inland, far), e[6], w, 0.0, "swamp");
        self.surface(Parameter::hull(t[3], t[4]), full, Parameter::hull(inland, far), e[6], w, 0.0, "mangrove_swamp");
        self.surface(frozen, full, Parameter::hull(inland, far), e[6], w, 0.0, "frozen_river");

        for ti in 0..5 {
            let t = self.temperatures[ti];
            for hi in 0..5 {
                let h = self.humidities[hi];
                let middle_or_badlands = self.pick_middle_or_badlands_if_hot(ti, hi, w);
                let (e, _coast, _near, mid, far) = self.bands();
                self.surface(t, h, Parameter::hull(mid, far), Parameter::hull(e[0], e[1]), w, 0.0, middle_or_badlands);
            }
        }
    }

    fn add_underground_biomes(&mut self) {
        let full = full_range();
        let (e, coast, _near, _mid, _far) = self.bands();
        let inland = self.inland;
        self.underground(full, full, Parameter::span(0.8, 1.0), full, full, 0.0, "dripstone_caves");
        self.underground(full, Parameter::span(0.7, 1.0), full, full, full, 0.0, "lush_caves");
        self.underground(
            full,
            full,
            Parameter::hull(coast, inland),
            Parameter::hull(e[5], e[6]),
            Parameter::span(-1.1, -0.85),
            0.0,
            "sulfur_caves",
        );
        self.bottom(full, full, full, Parameter::hull(e[0], e[1]), full, 0.0, "deep_dark");
    }

    // ---- the per-cell biome rules ----

    /// Convenience: the erosion bands and the four continentalness bands, which almost every
    /// call site needs together.
    fn bands(&self) -> ([Parameter; 7], Parameter, Parameter, Parameter, Parameter) {
        (self.erosions, self.coast, self.near_inland, self.mid_inland, self.far_inland)
    }

    /// `pickMiddleBiome` — the variant grid applies only on the positive-weirdness side.
    fn pick_middle(&self, ti: usize, hi: usize, w: Parameter) -> Biome {
        if w.max < 0 {
            MIDDLE_BIOMES[ti][hi]
        } else {
            MIDDLE_BIOMES_VARIANT[ti][hi].unwrap_or(MIDDLE_BIOMES[ti][hi])
        }
    }

    fn pick_middle_or_badlands_if_hot(&self, ti: usize, hi: usize, w: Parameter) -> Biome {
        if ti == 4 {
            pick_badlands(hi, w)
        } else {
            self.pick_middle(ti, hi, w)
        }
    }

    fn pick_middle_or_badlands_if_hot_or_slope_if_cold(&self, ti: usize, hi: usize, w: Parameter) -> Biome {
        if ti == 0 {
            self.pick_slope(ti, hi, w)
        } else {
            self.pick_middle_or_badlands_if_hot(ti, hi, w)
        }
    }

    /// `maybePickWindsweptSavannaBiome` — warm, not-too-humid, positive weirdness.
    fn maybe_windswept_savanna(&self, ti: usize, hi: usize, w: Parameter, underlying: Biome) -> Biome {
        if ti > 1 && hi < 4 && w.max >= 0 {
            "windswept_savanna"
        } else {
            underlying
        }
    }

    fn pick_shattered_coast(&self, ti: usize, hi: usize, w: Parameter) -> Biome {
        let base = if w.max >= 0 { self.pick_middle(ti, hi, w) } else { pick_beach(ti) };
        self.maybe_windswept_savanna(ti, hi, w, base)
    }

    fn pick_plateau(&self, ti: usize, hi: usize, w: Parameter) -> Biome {
        if w.max >= 0 {
            if let Some(variant) = PLATEAU_BIOMES_VARIANT[ti][hi] {
                return variant;
            }
        }
        PLATEAU_BIOMES[ti][hi]
    }

    fn pick_peak(&self, ti: usize, hi: usize, w: Parameter) -> Biome {
        if ti <= 2 {
            if w.max < 0 {
                "jagged_peaks"
            } else {
                "frozen_peaks"
            }
        } else if ti == 3 {
            "stony_peaks"
        } else {
            pick_badlands(hi, w)
        }
    }

    fn pick_slope(&self, ti: usize, hi: usize, w: Parameter) -> Biome {
        if ti >= 3 {
            self.pick_plateau(ti, hi, w)
        } else if hi <= 1 {
            "snowy_slopes"
        } else {
            "grove"
        }
    }

    fn pick_shattered(&self, ti: usize, hi: usize, w: Parameter) -> Biome {
        SHATTERED_BIOMES[ti][hi].unwrap_or_else(|| self.pick_middle(ti, hi, w))
    }
}

/// `pickBeachBiome` — snowy at the cold end, desert at the hot end.
fn pick_beach(ti: usize) -> Biome {
    match ti {
        0 => "snowy_beach",
        4 => "desert",
        _ => "beach",
    }
}

/// `pickBadlandsBiome`.
fn pick_badlands(hi: usize, w: Parameter) -> Biome {
    if hi < 2 {
        if w.max < 0 {
            "badlands"
        } else {
            "eroded_badlands"
        }
    } else if hi < 3 {
        "badlands"
    } else {
        "wooded_badlands"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn produces_the_expected_table_size() {
        // The datagen dump of 26.2's overworld has exactly this many boxes; if the port
        // drifts, this is the first thing that breaks. `examples/parity_biomes` checks the
        // entries themselves against that dump.
        let biomes = overworld_biomes();
        assert_eq!(biomes.len(), 7594, "biome box count");
        let distinct: HashSet<_> = biomes.iter().map(|(_, b)| *b).collect();
        assert_eq!(distinct.len(), 55, "distinct biomes");
    }

    #[test]
    fn surface_biomes_come_in_depth_pairs() {
        // The first entry is mushroom_fields at depth 0, immediately followed by depth 1.
        let biomes = overworld_biomes();
        assert_eq!(biomes[0].1, "mushroom_fields");
        assert_eq!(biomes[0].0.depth, Parameter::point(0.0));
        assert_eq!(biomes[1].1, "mushroom_fields");
        assert_eq!(biomes[1].0.depth, Parameter::point(1.0));
    }

    #[test]
    fn deep_dark_is_pinned_to_the_bottom_depth() {
        let biomes = overworld_biomes();
        let (point, _) = biomes.iter().find(|(_, b)| *b == "deep_dark").expect("deep_dark present");
        assert_eq!(point.depth, Parameter::point(1.1));
    }
}
