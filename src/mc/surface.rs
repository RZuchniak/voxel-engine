//! Surface rules — what the stone column actually gets *faced* with: grass, dirt, sand,
//! gravel, snow, terracotta banding, bedrock.
//!
//! Ports `SurfaceRules` (the condition/rule vocabulary), `SurfaceSystem` (the per-column
//! driver and its noises), and `SurfaceRuleData.overworld` (the ruleset itself).
//!
//! ## How it works
//!
//! The density + aquifer pass produces a column of stone/water/lava/air. This pass walks
//! that column top-down tracking three things — how deep into stone we are (`stone_depth_above`),
//! how far above the stone's bottom (`stone_depth_below`), and where the water surface is —
//! and for each *stone* block asks the rule tree what to put there. The tree is a
//! first-match-wins `sequence` of `ifTrue(condition, rule)` nodes, so ordering is the whole
//! semantics.
//!
//! Conditions come in two flavours that matter for correctness: those that depend only on
//! the column (`hole`, `steep`, surface depth) and those that also depend on Y
//! (`stone_depth_check`, `y_block_check`, water checks, 3d noise). Vanilla caches each kind
//! against a generation counter; here the driver simply recomputes, since the values are
//! cheap and the caching is not observable.

use std::sync::Arc;

use super::noise_params::{seed_factory, Noise};
use super::normal_noise::NormalNoise;
use super::overworld::Overworld;
use super::rng::PositionalFactory;

/// A block this generator can place. Only the overworld palette — the nether/end rules are
/// not ported.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Block {
    Air,
    Water,
    Lava,
    Stone,
    Deepslate,
    Bedrock,
    Dirt,
    CoarseDirt,
    Podzol,
    GrassBlock,
    Mycelium,
    Mud,
    Sand,
    Sandstone,
    RedSand,
    RedSandstone,
    Gravel,
    Calcite,
    PackedIce,
    Ice,
    SnowBlock,
    PowderSnow,
    Terracotta,
    WhiteTerracotta,
    OrangeTerracotta,
    YellowTerracotta,
    BrownTerracotta,
    RedTerracotta,
    LightGrayTerracotta,
    Cinnabar,
    Sulfur,
}

impl Block {
    /// The registry name, for comparing against a real world's blocks.
    pub fn name(self) -> &'static str {
        match self {
            Block::Air => "air",
            Block::Water => "water",
            Block::Lava => "lava",
            Block::Stone => "stone",
            Block::Deepslate => "deepslate",
            Block::Bedrock => "bedrock",
            Block::Dirt => "dirt",
            Block::CoarseDirt => "coarse_dirt",
            Block::Podzol => "podzol",
            Block::GrassBlock => "grass_block",
            Block::Mycelium => "mycelium",
            Block::Mud => "mud",
            Block::Sand => "sand",
            Block::Sandstone => "sandstone",
            Block::RedSand => "red_sand",
            Block::RedSandstone => "red_sandstone",
            Block::Gravel => "gravel",
            Block::Calcite => "calcite",
            Block::PackedIce => "packed_ice",
            Block::Ice => "ice",
            Block::SnowBlock => "snow_block",
            Block::PowderSnow => "powder_snow",
            Block::Terracotta => "terracotta",
            Block::WhiteTerracotta => "white_terracotta",
            Block::OrangeTerracotta => "orange_terracotta",
            Block::YellowTerracotta => "yellow_terracotta",
            Block::BrownTerracotta => "brown_terracotta",
            Block::RedTerracotta => "red_terracotta",
            Block::LightGrayTerracotta => "light_gray_terracotta",
            Block::Cinnabar => "cinnabar",
            Block::Sulfur => "sulfur",
        }
    }
}

/// Which side of a cave surface a depth check measures from (`CaveSurface`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum CaveSurface {
    Floor,
    Ceiling,
}

/// `SurfaceRules.ConditionSource` — a predicate over the current position.
enum Condition {
    /// `stoneDepthCheck(offset, addSurfaceDepth, secondaryDepthRange, surfaceType)`.
    StoneDepth { offset: i32, add_surface_depth: bool, secondary_depth_range: i32, surface: CaveSurface },
    /// `yBlockCheck` / `yStartCheck` — `add_stone_depth` distinguishes them.
    YCheck { anchor: i32, surface_depth_multiplier: i32, add_stone_depth: bool },
    /// `waterBlockCheck` / `waterStartCheck`.
    Water { offset: i32, surface_depth_multiplier: i32, add_stone_depth: bool },
    Biome(&'static [&'static str]),
    /// `noiseCondition2d` / `noiseCondition3d`.
    NoiseThreshold { noise: Noise, min: f64, max: f64, is_3d: bool },
    /// `verticalGradient` — a randomised fade between two heights, used for bedrock and the
    /// stone→deepslate transition.
    VerticalGradient { random_name: &'static str, true_at_and_below: i32, false_at_and_above: i32 },
    Steep,
    Hole,
    AbovePreliminarySurface,
    Temperature,
    Not(Box<Condition>),
}

/// `SurfaceRules.RuleSource` — produces a block, or nothing (fall through to the next rule).
enum Rule {
    /// `sequence(...)` — first rule that produces a block wins.
    Sequence(Vec<Rule>),
    /// `ifTrue(condition, then)`.
    IfTrue(Condition, Box<Rule>),
    /// `state(block)`.
    State(Block),
    /// `bandlands()` — the badlands terracotta banding.
    Bandlands,
}

/// The noises and derived state `SurfaceSystem` owns, built once per world.
pub struct SurfaceSystem {
    surface: Arc<NormalNoise>,
    surface_secondary: Arc<NormalNoise>,
    clay_bands_offset: Arc<NormalNoise>,
    /// Noises referenced by `noiseCondition` in the ruleset, resolved lazily by `Noise`.
    condition_noises: Vec<(Noise, Arc<NormalNoise>)>,
    clay_bands: Vec<Block>,
    /// `RandomState.random` — the top-level positional factory, used for surface depth
    /// jitter and by `verticalGradient`.
    noise_random: PositionalFactory,
    rule: Rule,
}

impl SurfaceSystem {
    pub fn new(seed: i64) -> Self {
        let factory = seed_factory(seed);
        let n = |noise: Noise| Arc::new(noise.instantiate(&factory));
        // `generateBands` consumes a dedicated stream, so its draw order is pinned.
        let clay_bands = generate_bands(&mut factory.from_hash_of("minecraft:clay_bands"));

        let rule = overworld_rule();
        let mut condition_noises = Vec::new();
        collect_noises(&rule, &mut condition_noises);
        let condition_noises =
            condition_noises.into_iter().map(|noise| (noise, n(noise))).collect();

        Self {
            surface: n(Noise::Surface),
            surface_secondary: n(Noise::SurfaceSecondary),
            clay_bands_offset: n(Noise::ClayBandsOffset),
            condition_noises,
            clay_bands,
            noise_random: factory,
            rule,
        }
    }

    /// `getSurfaceDepth` — the soil thickness for a column, noise plus a little jitter.
    fn surface_depth(&self, x: i32, z: i32) -> i32 {
        let noise = self.surface.get_value(x as f64, 0.0, z as f64);
        let jitter = self.noise_random.at(x, 0, z).next_double();
        (noise * 2.75 + 3.0 + jitter * 0.25) as i32
    }

    /// `getSurfaceSecondary`.
    fn surface_secondary(&self, x: i32, z: i32) -> f64 {
        self.surface_secondary.get_value(x as f64, 0.0, z as f64)
    }

    fn noise(&self, which: Noise) -> &NormalNoise {
        &self
            .condition_noises
            .iter()
            .find(|(n, _)| *n == which)
            .expect("every condition noise is collected at construction")
            .1
    }

    /// `getSurfaceDepth` — the soil thickness for a column.
    pub fn surface_depth_at(&self, x: i32, z: i32) -> i32 {
        self.surface_depth(x, z)
    }

    /// `Context.getMinSurfaceLevel` — the preliminary surface bilinearly interpolated across
    /// the 16-block surface cell, plus the soil thickness, minus 8. Below this the main
    /// close-to-surface rule is skipped entirely.
    pub fn min_surface_level(&self, ow: &Overworld, x: i32, z: i32, surface_depth: i32) -> i32 {
        let cell_x = x >> 4;
        let cell_z = z >> 4;
        let corner = |cx: i32, cz: i32| {
            ow.preliminary_surface_level((cx << 4) as f64, (cz << 4) as f64) as f32
        };
        let c00 = corner(cell_x, cell_z);
        let c10 = corner(cell_x + 1, cell_z);
        let c01 = corner(cell_x, cell_z + 1);
        let c11 = corner(cell_x + 1, cell_z + 1);
        let dx = (x & 15) as f32 / 16.0;
        let dz = (z & 15) as f32 / 16.0;
        let lerp = |t: f32, a: f32, b: f32| a + t * (b - a);
        let preliminary = lerp(dz, lerp(dx, c00, c10), lerp(dx, c01, c11)).floor() as i32;
        preliminary + surface_depth - 8
    }

    /// Evaluate the rule tree at one position. Returns the block to place, or `None` to
    /// leave the terrain block as it is.
    #[allow(clippy::too_many_arguments)]
    pub fn rule_at(
        &self,
        ow: &Overworld,
        x: i32,
        y: i32,
        z: i32,
        surface_depth: i32,
        min_surface_level: i32,
        stone_depth_above: i32,
        stone_depth_below: i32,
        water_height: i32,
        steep: bool,
        biome: &'static str,
    ) -> Option<Block> {
        let context = Context {
            system: self,
            ow,
            block_x: x,
            block_y: y,
            block_z: z,
            surface_depth,
            min_surface_level,
            stone_depth_above,
            stone_depth_below,
            water_height,
            steep,
            biome,
        };
        context.apply(&self.rule)
    }

    /// `getBand` — the terracotta band at a height, offset by a 2d noise.
    fn band(&self, x: i32, y: i32, z: i32) -> Block {
        let offset = (self.clay_bands_offset.get_value(x as f64, 0.0, z as f64) * 4.0).round() as i32;
        let len = self.clay_bands.len() as i32;
        self.clay_bands[((y + offset + len) % len) as usize]
    }
}

/// `generateBands` — build the 192-entry terracotta band table. Draw order is load-bearing.
fn generate_bands(random: &mut super::rng::XoroshiroRandom) -> Vec<Block> {
    let mut bands = vec![Block::Terracotta; 192];
    // Note Java mutates the loop variable inside the body, so this is a stride walk, not a
    // simple scan — `i += random.nextInt(5) + 1` happens *before* the write.
    let mut i = 0usize;
    while i < bands.len() {
        i += random.next_int_bound(5) as usize + 1;
        if i < bands.len() {
            bands[i] = Block::OrangeTerracotta;
        }
        i += 1;
    }
    make_bands(random, &mut bands, 1, Block::YellowTerracotta);
    make_bands(random, &mut bands, 2, Block::BrownTerracotta);
    make_bands(random, &mut bands, 1, Block::RedTerracotta);

    let white_band_count = random.next_int_between_inclusive(9, 15);
    let mut placed = 0;
    let mut start = 0usize;
    while placed < white_band_count && start < bands.len() {
        bands[start] = Block::WhiteTerracotta;
        if start >= 1 && random.next_boolean() {
            bands[start - 1] = Block::LightGrayTerracotta;
        }
        if start + 1 < bands.len() && random.next_boolean() {
            bands[start + 1] = Block::LightGrayTerracotta;
        }
        placed += 1;
        start += random.next_int_bound(16) as usize + 4;
    }
    bands
}

fn make_bands(random: &mut super::rng::XoroshiroRandom, bands: &mut [Block], base_width: i32, state: Block) {
    let count = random.next_int_between_inclusive(6, 15);
    for _ in 0..count {
        let width = base_width + random.next_int_bound(3);
        let start = random.next_int_bound(bands.len() as i32) as usize;
        for p in 0..width as usize {
            if start + p >= bands.len() {
                break;
            }
            bands[start + p] = state;
        }
    }
}

/// Walk the rule tree collecting every distinct noise a `NoiseThreshold` references.
fn collect_noises(rule: &Rule, out: &mut Vec<Noise>) {
    match rule {
        Rule::Sequence(rules) => rules.iter().for_each(|r| collect_noises(r, out)),
        Rule::IfTrue(condition, then) => {
            collect_condition_noises(condition, out);
            collect_noises(then, out);
        }
        Rule::State(_) | Rule::Bandlands => {}
    }
}

fn collect_condition_noises(condition: &Condition, out: &mut Vec<Noise>) {
    match condition {
        Condition::NoiseThreshold { noise, .. } => {
            if !out.contains(noise) {
                out.push(*noise);
            }
        }
        Condition::Not(inner) => collect_condition_noises(inner, out),
        _ => {}
    }
}

/// Everything the rule tree needs to know about the position being evaluated.
struct Context<'a> {
    system: &'a SurfaceSystem,
    ow: &'a Overworld,
    block_x: i32,
    block_y: i32,
    block_z: i32,
    /// Soil thickness for this column (`surfaceDepth`).
    surface_depth: i32,
    /// `minSurfaceLevel` — the preliminary surface, bilinearly interpolated over 16-block
    /// cells, plus surface depth, minus 8.
    min_surface_level: i32,
    /// Blocks of stone at and above this position (1 at the topmost stone block).
    stone_depth_above: i32,
    /// Blocks of stone from here down to the bottom of this stone run.
    stone_depth_below: i32,
    /// The water surface above this position, or `i32::MIN` for none.
    water_height: i32,
    /// Whether the column is steep here (a 4+ block height difference to a neighbour).
    steep: bool,
    biome: &'static str,
}

impl Context<'_> {
    fn test(&self, condition: &Condition) -> bool {
        match condition {
            Condition::StoneDepth { offset, add_surface_depth, secondary_depth_range, surface } => {
                let stone_depth = match surface {
                    CaveSurface::Ceiling => self.stone_depth_below,
                    CaveSurface::Floor => self.stone_depth_above,
                };
                let surface_depth = if *add_surface_depth { self.surface_depth } else { 0 };
                let secondary = if *secondary_depth_range == 0 {
                    0
                } else {
                    map(
                        self.system.surface_secondary(self.block_x, self.block_z),
                        -1.0,
                        1.0,
                        0.0,
                        *secondary_depth_range as f64,
                    ) as i32
                };
                stone_depth <= 1 + offset + surface_depth + secondary
            }
            Condition::YCheck { anchor, surface_depth_multiplier, add_stone_depth } => {
                let extra = if *add_stone_depth { self.stone_depth_above } else { 0 };
                self.block_y + extra >= anchor + self.surface_depth * surface_depth_multiplier
            }
            Condition::Water { offset, surface_depth_multiplier, add_stone_depth } => {
                if self.water_height == i32::MIN {
                    return true;
                }
                let extra = if *add_stone_depth { self.stone_depth_above } else { 0 };
                self.block_y + extra
                    >= self.water_height + offset + self.surface_depth * surface_depth_multiplier
            }
            Condition::Biome(names) => names.contains(&self.biome),
            Condition::NoiseThreshold { noise, min, max, is_3d } => {
                let y = if *is_3d { self.block_y as f64 } else { 0.0 };
                let v = self
                    .system
                    .noise(*noise)
                    .get_value(self.block_x as f64, y, self.block_z as f64);
                v >= *min && v <= *max
            }
            Condition::VerticalGradient { random_name, true_at_and_below, false_at_and_above } => {
                if self.block_y <= *true_at_and_below {
                    return true;
                }
                if self.block_y >= *false_at_and_above {
                    return false;
                }
                let probability = map(
                    self.block_y as f64,
                    *true_at_and_below as f64,
                    *false_at_and_above as f64,
                    1.0,
                    0.0,
                );
                let full = format!("minecraft:{random_name}");
                let mut random = self
                    .system
                    .noise_random
                    .from_hash_of(&full)
                    .fork_positional()
                    .at(self.block_x, self.block_y, self.block_z);
                (random.next_float() as f64) < probability
            }
            Condition::Steep => self.steep,
            Condition::Hole => self.surface_depth <= 0,
            Condition::AbovePreliminarySurface => self.block_y >= self.min_surface_level,
            Condition::Temperature => self.cold_enough_to_snow(),
            Condition::Not(inner) => !self.test(inner),
        }
    }

    /// `Biome.coldEnoughToSnow` — the biome's temperature at this height, below freezing.
    /// Vanilla reads the biome's registered base temperature and applies a height lapse
    /// above y=80; the biome's climate settings aren't ported, so this uses the climate
    /// temperature coordinate, which is the same signal the biome was chosen from.
    fn cold_enough_to_snow(&self) -> bool {
        let target = self.ow.climate_at_quart(self.block_x >> 2, self.block_y >> 2, self.block_z >> 2);
        // Frozen biomes occupy the lowest temperature band, max -0.45.
        target.temperature < super::climate::quantize(-0.45)
    }

    fn apply(&self, rule: &Rule) -> Option<Block> {
        match rule {
            Rule::Sequence(rules) => rules.iter().find_map(|r| self.apply(r)),
            Rule::IfTrue(condition, then) => {
                if self.test(condition) {
                    self.apply(then)
                } else {
                    None
                }
            }
            Rule::State(block) => Some(*block),
            Rule::Bandlands => Some(self.system.band(self.block_x, self.block_y, self.block_z)),
        }
    }
}

/// `Mth.map` — unclamped remap.
#[inline]
fn map(value: f64, from_min: f64, from_max: f64, to_min: f64, to_max: f64) -> f64 {
    to_min + (value - from_min) / (from_max - from_min) * (to_max - to_min)
}

// -------- the ruleset (`SurfaceRuleData.overworld`) --------

fn seq(rules: Vec<Rule>) -> Rule {
    Rule::Sequence(rules)
}
fn if_true(condition: Condition, then: Rule) -> Rule {
    Rule::IfTrue(condition, Box::new(then))
}
fn state(block: Block) -> Rule {
    Rule::State(block)
}
fn not(condition: Condition) -> Condition {
    Condition::Not(Box::new(condition))
}
fn y_block_check(anchor: i32, surface_depth_multiplier: i32) -> Condition {
    Condition::YCheck { anchor, surface_depth_multiplier, add_stone_depth: false }
}
fn y_start_check(anchor: i32, surface_depth_multiplier: i32) -> Condition {
    Condition::YCheck { anchor, surface_depth_multiplier, add_stone_depth: true }
}
fn water_block_check(offset: i32, surface_depth_multiplier: i32) -> Condition {
    Condition::Water { offset, surface_depth_multiplier, add_stone_depth: false }
}
fn water_start_check(offset: i32, surface_depth_multiplier: i32) -> Condition {
    Condition::Water { offset, surface_depth_multiplier, add_stone_depth: true }
}
fn noise_2d(noise: Noise, min: f64, max: f64) -> Condition {
    Condition::NoiseThreshold { noise, min, max, is_3d: false }
}
fn noise_3d(noise: Noise, min: f64, max: f64) -> Condition {
    Condition::NoiseThreshold { noise, min, max, is_3d: true }
}
/// `surfaceNoiseAbove(threshold)`.
fn surface_noise_above(threshold: f64) -> Condition {
    noise_2d(Noise::Surface, threshold / 8.25, f64::MAX)
}
fn biome(names: &'static [&'static str]) -> Condition {
    Condition::Biome(names)
}

/// `SurfaceRules.ON_FLOOR` etc.
fn on_floor() -> Condition {
    Condition::StoneDepth { offset: 0, add_surface_depth: false, secondary_depth_range: 0, surface: CaveSurface::Floor }
}
fn under_floor() -> Condition {
    Condition::StoneDepth { offset: 0, add_surface_depth: true, secondary_depth_range: 0, surface: CaveSurface::Floor }
}
fn deep_under_floor() -> Condition {
    Condition::StoneDepth { offset: 0, add_surface_depth: true, secondary_depth_range: 6, surface: CaveSurface::Floor }
}
fn very_deep_under_floor() -> Condition {
    Condition::StoneDepth { offset: 0, add_surface_depth: true, secondary_depth_range: 30, surface: CaveSurface::Floor }
}
fn on_ceiling() -> Condition {
    Condition::StoneDepth { offset: 0, add_surface_depth: false, secondary_depth_range: 0, surface: CaveSurface::Ceiling }
}

/// `SurfaceRuleData.overworld()` — `overworldLike(biomes, true, false, true)`.
fn overworld_rule() -> Rule {
    use Block::*;

    let wooded_badlands_top = y_block_check(97, 2);
    let badlands_top = y_block_check(256, 0);
    let badlands_height_condition = y_start_check(63, -1);
    let badlands_mid = || y_start_check(74, 1);
    let mangrove_swamp_puddle_level = y_block_check(60, 0);
    let swamp_puddle_level = y_block_check(62, 0);
    let above_sea_level = || y_block_check(63, 0);
    let not_underwater = || water_block_check(-1, 0);
    let above_water = || water_block_check(0, 0);
    let not_under_deep_water = || water_start_check(-6, -1);
    let frozen_ocean = || biome(&["frozen_ocean", "deep_frozen_ocean"]);

    let grass_or_dirt_if_underwater =
        || seq(vec![if_true(above_water(), state(GrassBlock)), state(Dirt)]);
    let sand_or_sandstone_if_ceiling =
        || seq(vec![if_true(on_ceiling(), state(Sandstone)), state(Sand)]);
    let gravel_or_stone_if_ceiling = || seq(vec![if_true(on_ceiling(), state(Stone)), state(Gravel)]);
    let biomes_with_sand_and_sandstone = || biome(&["warm_ocean", "beach", "snowy_beach"]);
    let biomes_with_sand_and_very_deep_sandstone = || biome(&["desert"]);

    let sulfur_cave_bands = || {
        seq(vec![
            if_true(noise_3d(Noise::SulfurCaveGradient, -0.4f32 as f64, -0.1f32 as f64), state(Cinnabar)),
            if_true(noise_3d(Noise::SulfurCaveGradient, 0.0, 0.4f32 as f64), state(Sulfur)),
            if_true(noise_3d(Noise::SulfurCaveGradient, 0.4f32 as f64, f64::MAX), state(Cinnabar)),
        ])
    };

    let common_surface_and_under_rules = || {
        seq(vec![
            if_true(
                biome(&["stony_peaks"]),
                seq(vec![if_true(noise_2d(Noise::Calcite, -0.0125, 0.0125), state(Calcite)), state(Stone)]),
            ),
            if_true(
                biome(&["stony_shore"]),
                seq(vec![
                    if_true(noise_2d(Noise::Gravel, -0.05, 0.05), gravel_or_stone_if_ceiling()),
                    state(Stone),
                ]),
            ),
            if_true(biome(&["windswept_hills"]), if_true(surface_noise_above(1.0), state(Stone))),
            if_true(biomes_with_sand_and_sandstone(), sand_or_sandstone_if_ceiling()),
            if_true(biomes_with_sand_and_very_deep_sandstone(), sand_or_sandstone_if_ceiling()),
            if_true(biome(&["dripstone_caves"]), state(Stone)),
            if_true(biome(&["sulfur_caves"]), seq(vec![sulfur_cave_bands(), state(Stone)])),
        ])
    };

    let powder_snow_under_rule =
        || if_true(noise_2d(Noise::PowderSnow, 0.45, 0.58), if_true(above_water(), state(PowderSnow)));
    let powder_snow_surface_rule =
        || if_true(noise_2d(Noise::PowderSnow, 0.35, 0.6), if_true(above_water(), state(PowderSnow)));

    let biome_under_surface_rule = seq(vec![
        if_true(
            biome(&["frozen_peaks"]),
            seq(vec![
                if_true(Condition::Steep, state(PackedIce)),
                if_true(noise_2d(Noise::PackedIce, -0.5, 0.2), state(PackedIce)),
                if_true(noise_2d(Noise::Ice, -0.0625, 0.025), state(Ice)),
                if_true(above_water(), state(SnowBlock)),
            ]),
        ),
        if_true(
            biome(&["snowy_slopes"]),
            seq(vec![
                if_true(Condition::Steep, state(Stone)),
                powder_snow_under_rule(),
                if_true(above_water(), state(SnowBlock)),
            ]),
        ),
        if_true(biome(&["jagged_peaks"]), state(Stone)),
        if_true(biome(&["grove"]), seq(vec![powder_snow_under_rule(), state(Dirt)])),
        common_surface_and_under_rules(),
        if_true(biome(&["windswept_savanna"]), if_true(surface_noise_above(1.75), state(Stone))),
        if_true(
            biome(&["windswept_gravelly_hills"]),
            seq(vec![
                if_true(surface_noise_above(2.0), gravel_or_stone_if_ceiling()),
                if_true(surface_noise_above(1.0), state(Stone)),
                if_true(surface_noise_above(-1.0), state(Dirt)),
                gravel_or_stone_if_ceiling(),
            ]),
        ),
        if_true(biome(&["mangrove_swamp"]), state(Mud)),
        state(Dirt),
    ]);

    let biome_surface_rule = seq(vec![
        if_true(
            biome(&["frozen_peaks"]),
            seq(vec![
                if_true(Condition::Steep, state(PackedIce)),
                if_true(noise_2d(Noise::PackedIce, 0.0, 0.2), state(PackedIce)),
                if_true(noise_2d(Noise::Ice, 0.0, 0.025), state(Ice)),
                if_true(above_water(), state(SnowBlock)),
            ]),
        ),
        if_true(
            biome(&["snowy_slopes"]),
            seq(vec![
                if_true(Condition::Steep, state(Stone)),
                powder_snow_surface_rule(),
                if_true(above_water(), state(SnowBlock)),
            ]),
        ),
        if_true(
            biome(&["jagged_peaks"]),
            seq(vec![if_true(Condition::Steep, state(Stone)), if_true(above_water(), state(SnowBlock))]),
        ),
        if_true(
            biome(&["grove"]),
            seq(vec![powder_snow_surface_rule(), if_true(above_water(), state(SnowBlock))]),
        ),
        common_surface_and_under_rules(),
        if_true(
            biome(&["windswept_savanna"]),
            seq(vec![
                if_true(surface_noise_above(1.75), state(Stone)),
                if_true(surface_noise_above(-0.5), state(CoarseDirt)),
            ]),
        ),
        if_true(
            biome(&["windswept_gravelly_hills"]),
            seq(vec![
                if_true(surface_noise_above(2.0), gravel_or_stone_if_ceiling()),
                if_true(surface_noise_above(1.0), state(Stone)),
                if_true(surface_noise_above(-1.0), grass_or_dirt_if_underwater()),
                gravel_or_stone_if_ceiling(),
            ]),
        ),
        if_true(
            biome(&["old_growth_pine_taiga", "old_growth_spruce_taiga"]),
            seq(vec![
                if_true(surface_noise_above(1.75), state(CoarseDirt)),
                if_true(surface_noise_above(-0.95), state(Podzol)),
            ]),
        ),
        if_true(biome(&["ice_spikes"]), if_true(above_water(), state(SnowBlock))),
        if_true(biome(&["mangrove_swamp"]), state(Mud)),
        if_true(biome(&["mushroom_fields"]), state(Mycelium)),
        grass_or_dirt_if_underwater(),
    ]);

    let clay_band_1 = || noise_2d(Noise::Surface, -0.909, -0.5454);
    let clay_band_2 = || noise_2d(Noise::Surface, -0.1818, 0.1818);
    let clay_band_3 = || noise_2d(Noise::Surface, 0.5454, 0.909);

    let main_rule_close_to_surface = seq(vec![
        if_true(
            on_floor(),
            seq(vec![
                if_true(
                    biome(&["wooded_badlands"]),
                    if_true(
                        wooded_badlands_top,
                        seq(vec![
                            if_true(clay_band_1(), state(CoarseDirt)),
                            if_true(clay_band_2(), state(CoarseDirt)),
                            if_true(clay_band_3(), state(CoarseDirt)),
                            grass_or_dirt_if_underwater(),
                        ]),
                    ),
                ),
                if_true(
                    biome(&["swamp"]),
                    if_true(
                        swamp_puddle_level,
                        if_true(
                            not(above_sea_level()),
                            if_true(noise_2d(Noise::Swamp, 0.0, f64::MAX), state(Water)),
                        ),
                    ),
                ),
                if_true(
                    biome(&["mangrove_swamp"]),
                    if_true(
                        mangrove_swamp_puddle_level,
                        if_true(
                            not(above_sea_level()),
                            if_true(noise_2d(Noise::Swamp, 0.0, f64::MAX), state(Water)),
                        ),
                    ),
                ),
            ]),
        ),
        if_true(
            biome(&["badlands", "eroded_badlands", "wooded_badlands"]),
            seq(vec![
                if_true(
                    on_floor(),
                    seq(vec![
                        if_true(badlands_top, state(OrangeTerracotta)),
                        if_true(
                            badlands_mid(),
                            seq(vec![
                                if_true(clay_band_1(), state(Terracotta)),
                                if_true(clay_band_2(), state(Terracotta)),
                                if_true(clay_band_3(), state(Terracotta)),
                                Rule::Bandlands,
                            ]),
                        ),
                        if_true(
                            not_underwater(),
                            seq(vec![if_true(on_ceiling(), state(RedSandstone)), state(RedSand)]),
                        ),
                        if_true(not(Condition::Hole), state(OrangeTerracotta)),
                        if_true(not_under_deep_water(), state(WhiteTerracotta)),
                        gravel_or_stone_if_ceiling(),
                    ]),
                ),
                if_true(
                    badlands_height_condition,
                    seq(vec![
                        if_true(above_sea_level(), if_true(not(badlands_mid()), state(OrangeTerracotta))),
                        Rule::Bandlands,
                    ]),
                ),
                if_true(under_floor(), if_true(not_under_deep_water(), state(WhiteTerracotta))),
            ]),
        ),
        if_true(
            on_floor(),
            if_true(
                not_underwater(),
                seq(vec![
                    if_true(
                        frozen_ocean(),
                        if_true(
                            Condition::Hole,
                            seq(vec![
                                if_true(above_water(), state(Air)),
                                if_true(Condition::Temperature, state(Ice)),
                                state(Water),
                            ]),
                        ),
                    ),
                    biome_surface_rule,
                ]),
            ),
        ),
        if_true(
            not_under_deep_water(),
            seq(vec![
                if_true(on_floor(), if_true(frozen_ocean(), if_true(Condition::Hole, state(Water)))),
                if_true(under_floor(), biome_under_surface_rule),
                if_true(biomes_with_sand_and_sandstone(), if_true(deep_under_floor(), state(Sandstone))),
                if_true(
                    biomes_with_sand_and_very_deep_sandstone(),
                    if_true(very_deep_under_floor(), state(Sandstone)),
                ),
            ]),
        ),
        if_true(
            on_floor(),
            seq(vec![
                if_true(biome(&["frozen_peaks", "jagged_peaks"]), state(Stone)),
                if_true(
                    biome(&["warm_ocean", "lukewarm_ocean", "deep_lukewarm_ocean"]),
                    sand_or_sandstone_if_ceiling(),
                ),
                gravel_or_stone_if_ceiling(),
            ]),
        ),
    ]);

    // bedrockRoof = false, bedrockFloor = true, doPreliminarySurfaceCheck = true.
    seq(vec![
        if_true(
            Condition::VerticalGradient {
                random_name: "bedrock_floor",
                true_at_and_below: -64,
                false_at_and_above: -59,
            },
            state(Bedrock),
        ),
        if_true(Condition::AbovePreliminarySurface, main_rule_close_to_surface),
        if_true(biome(&["sulfur_caves"]), sulfur_cave_bands()),
        if_true(
            Condition::VerticalGradient {
                random_name: "deepslate",
                true_at_and_below: 0,
                false_at_and_above: 8,
            },
            state(Deepslate),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: i64 = 6954908675375307936;

    #[test]
    fn clay_bands_are_generated_and_varied() {
        let system = SurfaceSystem::new(SEED);
        assert_eq!(system.clay_bands.len(), 192);
        // The table must contain more than just the terracotta fill.
        let distinct: std::collections::HashSet<_> = system.clay_bands.iter().collect();
        assert!(distinct.len() >= 4, "clay bands only produced {} kinds", distinct.len());
        assert!(distinct.contains(&Block::WhiteTerracotta), "white bands missing");
    }

    #[test]
    fn every_condition_noise_resolves() {
        // `SurfaceSystem::noise` panics on an unregistered noise; building the system and
        // walking every condition proves the collect step found them all.
        let system = SurfaceSystem::new(SEED);
        assert!(!system.condition_noises.is_empty());
        for (noise, _) in &system.condition_noises {
            let _ = system.noise(*noise);
        }
    }

    #[test]
    fn surface_depth_is_a_small_positive_thickness() {
        let system = SurfaceSystem::new(SEED);
        for (x, z) in [(0, 0), (37, -21), (128, 256)] {
            let depth = system.surface_depth(x, z);
            assert!((0..=8).contains(&depth), "surface depth {depth} at ({x},{z}) out of range");
        }
    }
}
