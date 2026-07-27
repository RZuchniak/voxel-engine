//! Feature decoration — currently trees only.
//!
//! Ports the parts of `ChunkGenerator.applyBiomeDecoration` and the
//! `net.minecraft.world.level.levelgen.placement` modifier chain that trees go through, plus the
//! per-biome tree tables from `VegetationPlacements` / `BiomeDefaultFeatures`.
//!
//! # Why decoration is a 3×3 pass
//!
//! `InSquarePlacement` puts a trunk anywhere in a chunk's own 16×16, and foliage reaches about
//! three blocks past it. So a chunk's final contents depend on its **neighbours'** decoration as
//! well as its own, and generating chunk C means running decoration for all nine chunks of its
//! neighbourhood and keeping only the writes that land in C.
//!
//! Done naively that regenerates each chunk's terrain nine times (~37.6 ms each). The terrain
//! cache in `source::SeededProceduralSource` is what makes it affordable — see the perf table in
//! HANDOFF.md. **Do not "optimise" this by decorating only the centre chunk and clipping**: that
//! leaves a sawn-off half tree on every chunk border, which is far worse than no trees.
//!
//! # What is exact here, and what is not
//!
//! Exact: the tree geometry (`mc::tree`), the RNG (`mc::wgrandom`), the modifier chain's
//! arithmetic and **draw order**, and the per-biome frequencies.
//!
//! ⚠️ **Not exact: the feature seed's `index` argument.** Vanilla passes a registry-wide index
//! produced by `FeatureSorter.buildFeaturesPerStep`, a topological sort over *every* biome's
//! complete feature list — ores, lakes and all. Reproducing it means porting `OverworldBiomes`
//! and `BiomeDefaultFeatures` in full so that unimplemented features still occupy their slots.
//! Until then [`TreeKind::feature_index`] is this module's own stable numbering, so **tree
//! positions do not match a real save**, even though their shapes, species and densities do.
//! That is the single remaining gap to bit-exact trees.

use super::chunk::{ChunkBlocks, MIN_Y};
use super::surface::Block;
use super::tree::{self, TreeCanvas, TreeConfig};
use super::wgrandom::WorldgenRandom;

/// `GenerationStep.Decoration.VEGETAL_DECORATION.ordinal()`. Trees are seeded with this as the
/// `step` argument to `setFeatureSeed`.
const VEGETAL_DECORATION: i32 = 9;

/// `PlacementUtils.countExtra(count, chance, extra)`.
///
/// A `WeightedListInt` over two constants: `1/chance - 1` parts `count` and one part
/// `count + extra`. `WeightedList.getRandom` draws a single `nextInt(totalWeight)`, so this
/// costs **one** draw regardless of the outcome.
#[derive(Debug, Clone, Copy)]
struct CountExtra {
    count: i32,
    /// `1 / chance`, i.e. the weighted list's total weight.
    total_weight: i32,
    extra: i32,
}

impl CountExtra {
    const fn new(count: i32, one_over_chance: i32, extra: i32) -> Self {
        Self { count, total_weight: one_over_chance, extra }
    }

    fn sample(&self, random: &mut WorldgenRandom) -> i32 {
        // The flat selector lays the weights out in order, so the single high-weight entry
        // (`count + extra`) occupies exactly the last slot.
        let selection = random.next_int_bound(self.total_weight);
        if selection == self.total_weight - 1 {
            self.count + self.extra
        } else {
            self.count
        }
    }
}

/// One entry of a `RandomFeatureConfiguration` — a species and the chance of choosing it.
#[derive(Debug, Clone, Copy)]
struct WeightedSpecies {
    chance: f32,
    config: TreeConfig,
}

/// A biome's tree recipe: how many attempts per chunk, and which species.
#[derive(Debug, Clone, Copy)]
pub struct TreeRecipe {
    frequency: CountExtra,
    /// Tried in order; the first whose `nextFloat() < chance` wins (`RandomSelectorFeature`).
    weighted: &'static [WeightedSpecies],
    /// Used when no weighted entry is selected.
    default: TreeConfig,
}

/// The tree placements a biome can use. One variant per distinct `VegetationPlacements.TREES_*`
/// that this port covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeKind {
    Plains,
    Forest,
    BirchForest,
    Taiga,
    WindsweptHills,
    Savanna,
    Snowy,
    Grove,
}

impl TreeKind {
    /// All kinds, in a fixed order — this is the order decoration runs them in, so it must be
    /// stable for the world to be reproducible.
    const ALL: [TreeKind; 8] = [
        TreeKind::Plains,
        TreeKind::Forest,
        TreeKind::BirchForest,
        TreeKind::Taiga,
        TreeKind::WindsweptHills,
        TreeKind::Savanna,
        TreeKind::Snowy,
        TreeKind::Grove,
    ];

    /// ⚠️ **This module's own numbering, not Minecraft's.** See the module docs: vanilla's index
    /// comes from a topological sort over every biome's full feature list. Stable and distinct is
    /// all that is required for a self-consistent world; matching a real save needs the real one.
    fn feature_index(self) -> i32 {
        match self {
            TreeKind::Plains => 0,
            TreeKind::Forest => 1,
            TreeKind::BirchForest => 2,
            TreeKind::Taiga => 3,
            TreeKind::WindsweptHills => 4,
            TreeKind::Savanna => 5,
            TreeKind::Snowy => 6,
            TreeKind::Grove => 7,
        }
    }

    fn recipe(self) -> TreeRecipe {
        match self {
            // `TREES_PLAINS`: countExtra(0, 0.05, 1) — one tree per 20 chunks on average, which
            // is why plains reads as open grassland with the occasional lone oak.
            TreeKind::Plains => TreeRecipe {
                frequency: CountExtra::new(0, 20, 1),
                weighted: &[],
                default: tree::OAK,
            },
            // `TREES_BIRCH_AND_OAK`: countExtra(10, 0.1, 1), birch mixed into oak.
            TreeKind::Forest => TreeRecipe {
                frequency: CountExtra::new(10, 10, 1),
                weighted: &[WeightedSpecies { chance: 0.2, config: tree::BIRCH }],
                default: tree::OAK,
            },
            // `TREES_BIRCH`: countExtra(10, 0.1, 1), pure birch.
            TreeKind::BirchForest => TreeRecipe {
                frequency: CountExtra::new(10, 10, 1),
                weighted: &[],
                default: tree::BIRCH,
            },
            // `TREES_TAIGA`: countExtra(10, 0.1, 1) — pine a third of the time, else spruce.
            TreeKind::Taiga => TreeRecipe {
                frequency: CountExtra::new(10, 10, 1),
                weighted: &[WeightedSpecies { chance: 0.333_333_34, config: tree::PINE }],
                default: tree::SPRUCE,
            },
            // `TREES_WINDSWEPT_HILLS`: countExtra(0, 0.1, 1), spruce with some oak.
            TreeKind::WindsweptHills => TreeRecipe {
                frequency: CountExtra::new(0, 10, 1),
                weighted: &[WeightedSpecies { chance: 0.666, config: tree::SPRUCE }],
                default: tree::OAK,
            },
            // `TREES_SAVANNA`: countExtra(1, 0.1, 1). Vanilla uses acacia, which is not ported —
            // oak stands in, so savanna gets the right *density* with the wrong silhouette.
            TreeKind::Savanna => TreeRecipe {
                frequency: CountExtra::new(1, 10, 1),
                weighted: &[],
                default: tree::OAK,
            },
            // `TREES_SNOWY`: countExtra(0, 0.1, 1), sparse spruce.
            TreeKind::Snowy => TreeRecipe {
                frequency: CountExtra::new(0, 10, 1),
                weighted: &[],
                default: tree::SPRUCE,
            },
            // `TREES_GROVE`: countExtra(10, 0.1, 1), spruce/pine like taiga.
            TreeKind::Grove => TreeRecipe {
                frequency: CountExtra::new(10, 10, 1),
                weighted: &[WeightedSpecies { chance: 0.333_333_34, config: tree::PINE }],
                default: tree::SPRUCE,
            },
        }
    }
}

/// Which tree placement a biome uses, from `BiomeDefaultFeatures` / `OverworldBiomes`.
///
/// `None` means the biome grows no trees this port covers — oceans, deserts, badlands (which use
/// their own sparse oak variant), the cave biomes, and the jungle/swamp/dark-forest/cherry
/// families whose species are not ported yet.
pub fn tree_kind_for_biome(biome: &str) -> Option<TreeKind> {
    Some(match biome {
        "plains" | "sunflower_plains" | "meadow" => TreeKind::Plains,
        "forest" | "flower_forest" => TreeKind::Forest,
        "birch_forest" | "old_growth_birch_forest" => TreeKind::BirchForest,
        "taiga" | "old_growth_pine_taiga" | "old_growth_spruce_taiga" => TreeKind::Taiga,
        "windswept_hills" | "windswept_gravelly_hills" | "windswept_forest" => {
            TreeKind::WindsweptHills
        }
        "savanna" | "savanna_plateau" | "windswept_savanna" => TreeKind::Savanna,
        "snowy_plains" | "snowy_taiga" | "snowy_slopes" | "ice_spikes" => TreeKind::Snowy,
        "grove" => TreeKind::Grove,
        _ => return None,
    })
}

/// Chunks per side of the decoration neighbourhood.
const NEIGHBOURHOOD: i32 = 3;
/// Blocks per side of the canvas.
const CANVAS_SIDE: i32 = NEIGHBOURHOOD * 16;

/// A 3×3-chunk block volume that trees are grown into.
///
/// Reads outside the volume return [`Block::Stone`], which blocks tree growth rather than
/// silently letting it succeed on unknown ground — the conservative direction, since a tree that
/// fails to place leaves the world unchanged while one that places on nothing leaves it floating.
pub struct Canvas {
    blocks: Vec<Block>,
    /// World coords of the canvas's minimum corner.
    origin_x: i32,
    origin_z: i32,
    height: i32,
}

impl Canvas {
    fn index(&self, x: i32, y: i32, z: i32) -> Option<usize> {
        let lx = x - self.origin_x;
        let lz = z - self.origin_z;
        let ly = y - MIN_Y;
        if lx < 0 || lz < 0 || ly < 0 || lx >= CANVAS_SIDE || lz >= CANVAS_SIDE || ly >= self.height
        {
            return None;
        }
        Some(((ly * CANVAS_SIDE + lz) * CANVAS_SIDE + lx) as usize)
    }
}

impl TreeCanvas for Canvas {
    fn get(&self, x: i32, y: i32, z: i32) -> Block {
        self.index(x, y, z).map_or(Block::Stone, |i| self.blocks[i])
    }
    fn set(&mut self, x: i32, y: i32, z: i32, block: Block) {
        if let Some(i) = self.index(x, y, z) {
            self.blocks[i] = block;
        }
    }
    fn min_y(&self) -> i32 {
        MIN_Y
    }
    fn max_y(&self) -> i32 {
        MIN_Y + self.height - 1
    }
}

/// The terrain of one chunk, as decoration needs to see it.
pub struct NeighbourTerrain<'a> {
    pub chunk_x: i32,
    pub chunk_z: i32,
    pub blocks: &'a ChunkBlocks,
}

/// Decorate the 3×3 neighbourhood around `(centre_x, centre_z)` and return the centre chunk's
/// blocks with trees written in.
///
/// `neighbours` must contain all nine chunks; anything missing is simply not decorated, which
/// loses that chunk's trees rather than producing wrong ones.
pub fn decorate_centre(
    seed: i64,
    centre_x: i32,
    centre_z: i32,
    neighbours: &[NeighbourTerrain<'_>],
) -> Vec<Block> {
    let height = super::chunk::HEIGHT;
    let origin_x = (centre_x - 1) * 16;
    let origin_z = (centre_z - 1) * 16;
    let mut canvas = Canvas {
        blocks: vec![Block::Stone; (CANVAS_SIDE * CANVAS_SIDE * height) as usize],
        origin_x,
        origin_z,
        height,
    };

    // Load the nine chunks' terrain into the canvas.
    for neighbour in neighbours {
        let base_x = neighbour.chunk_x * 16;
        let base_z = neighbour.chunk_z * 16;
        for lz in 0..16usize {
            for lx in 0..16usize {
                for y in MIN_Y..MIN_Y + height {
                    let block = neighbour.blocks.get(lx, y, lz);
                    canvas.set(base_x + lx as i32, y, base_z + lz as i32, block);
                }
            }
        }
    }

    // Decorate every chunk of the neighbourhood.
    //
    // ⚠️ Each chunk's trees are computed against **pristine terrain plus only its own trees so
    // far**, never against other chunks' trees, and are merged in afterwards. That isolation is
    // required, not tidiness: chunk N is decorated both when generating N and when generating
    // each of its 8 neighbours, and those runs see *different* neighbourhoods. If a tree's
    // clearance check could see a neighbouring chunk's canopy, N would grow different trees
    // depending on which chunk was being generated — so a canopy would appear on one side of a
    // border and not the other. That is exactly what
    // `correctness_trees::a_canopy_crossing_a_chunk_border_is_not_cut_off` caught.
    //
    // This is a deliberate divergence from vanilla, which decorates chunks in world-generation
    // order and does let a later chunk see an earlier one's trees. Reproducing that needs
    // generation *order* as an input, which a stateless parallel `WorldSource` does not have.
    // The visible cost is that trees from adjacent chunks may interpenetrate slightly rather
    // than yielding to each other.
    for neighbour in neighbours {
        let overlay = decorate_chunk(seed, neighbour, &canvas);
        for ((x, y, z), block) in overlay {
            canvas.set(x, y, z, block);
        }
    }

    // Extract the centre column.
    let base_x = centre_x * 16;
    let base_z = centre_z * 16;
    let mut out = vec![Block::Air; (16 * 16 * height) as usize];
    for lz in 0..16usize {
        for lx in 0..16usize {
            for y in MIN_Y..MIN_Y + height {
                let block = canvas.get(base_x + lx as i32, y, base_z + lz as i32);
                out[((y - MIN_Y) as usize) * 256 + lz * 16 + lx] = block;
            }
        }
    }
    out
}

/// A chunk's own tree blocks, before they are merged into the shared canvas.
///
/// Sparse because trees are: a chunk holds a handful of them against 98304 blocks.
type Overlay = Vec<((i32, i32, i32), Block)>;

/// Reads pristine terrain plus this chunk's own trees; writes only to the overlay.
///
/// The split is what makes a chunk's decoration independent of which neighbourhood it is being
/// decorated in — see the note in [`decorate_centre`].
struct OverlayCanvas<'a> {
    base: &'a Canvas,
    written: std::collections::HashMap<(i32, i32, i32), Block>,
    overlay: Overlay,
}

impl TreeCanvas for OverlayCanvas<'_> {
    fn get(&self, x: i32, y: i32, z: i32) -> Block {
        match self.written.get(&(x, y, z)) {
            Some(block) => *block,
            None => self.base.get(x, y, z),
        }
    }
    fn set(&mut self, x: i32, y: i32, z: i32, block: Block) {
        // Outside the canvas the write is dropped rather than recorded: it cannot belong to the
        // centre chunk, and keeping it would let a tree at the far edge of a corner neighbour
        // grow through terrain this canvas cannot see.
        if self.base.index(x, y, z).is_none() {
            return;
        }
        self.written.insert((x, y, z), block);
        self.overlay.push(((x, y, z), block));
    }
    fn min_y(&self) -> i32 {
        self.base.min_y()
    }
    fn max_y(&self) -> i32 {
        self.base.max_y()
    }
}

/// Run one chunk's vegetal decoration, returning the blocks it wants to place.
///
/// Mirrors `applyBiomeDecoration`'s inner loop: seed once per chunk from the block origin, then
/// per feature re-seed with `setFeatureSeed` before running its placement chain.
fn decorate_chunk(seed: i64, neighbour: &NeighbourTerrain<'_>, base: &Canvas) -> Overlay {
    let origin_x = neighbour.chunk_x * 16;
    let origin_z = neighbour.chunk_z * 16;

    let mut random = WorldgenRandom::from_seed(0);
    let decoration_seed = random.set_decoration_seed(seed, origin_x, origin_z);

    // Which tree features this chunk can run. Vanilla takes the union over every biome in the
    // chunk's own 3×3 neighbourhood; this uses the chunk's own 4×4 biome grid, which differs
    // only for a feature belonging solely to a biome that does not reach this chunk at all —
    // and `BiomeFilter` below would reject every position it produced anyway.
    let mut kinds: Vec<TreeKind> = Vec::new();
    for biome in neighbour.blocks.surface_biomes.iter() {
        if let Some(kind) = tree_kind_for_biome(biome) {
            if !kinds.contains(&kind) {
                kinds.push(kind);
            }
        }
    }

    // Run in a fixed global order, not discovery order, so the world does not depend on how the
    // biome grid happened to be laid out.
    let mut canvas = OverlayCanvas {
        base,
        written: std::collections::HashMap::new(),
        overlay: Vec::new(),
    };
    for kind in TreeKind::ALL {
        if !kinds.contains(&kind) {
            continue;
        }
        random.set_feature_seed(decoration_seed, kind.feature_index(), VEGETAL_DECORATION);
        place_trees(&mut random, kind, neighbour, &mut canvas, origin_x, origin_z);
    }
    canvas.overlay
}

/// The `treePlacement` modifier chain, in vanilla's evaluation order.
///
/// Java builds this as a lazy `Stream` of `flatMap`s, which is **depth-first**: the count is
/// drawn once, then each position runs the whole remaining chain — including the tree's own
/// draws — before the next position is generated. Reordering this into "generate all positions,
/// then place all trees" would consume the random in a different order and move every tree.
fn place_trees(
    random: &mut WorldgenRandom,
    kind: TreeKind,
    neighbour: &NeighbourTerrain<'_>,
    canvas: &mut OverlayCanvas<'_>,
    origin_x: i32,
    origin_z: i32,
) {
    let recipe = kind.recipe();
    let count = recipe.frequency.sample(random);

    for _ in 0..count {
        // `InSquarePlacement.spread()`.
        let x = random.next_int_bound(16) + origin_x;
        let z = random.next_int_bound(16) + origin_z;

        let lx = (x - origin_x) as usize;
        let lz = (z - origin_z) as usize;
        let column = lz * 16 + lx;
        let ocean_floor = neighbour.blocks.ocean_floor[column];
        let world_surface = neighbour.blocks.world_surface[column];

        // `TREE_THRESHOLD` = `SurfaceWaterDepthFilter.forMaxDepth(0)` — no standing water.
        if world_surface - ocean_floor > 0 {
            continue;
        }
        // `PlacementUtils.HEIGHTMAP_OCEAN_FLOOR`.
        if ocean_floor <= MIN_Y {
            continue;
        }
        // `BiomeFilter.biome()` — the biome *at the sampled position* must be one that runs this
        // feature. Without this a chunk straddling forest and ocean would grow trees in the sea.
        let qx = lx / 4;
        let qz = lz / 4;
        let biome = neighbour.blocks.surface_biomes[qz * 4 + qx];
        if tree_kind_for_biome(biome) != Some(kind) {
            continue;
        }
        // A tree only grows on ground a sapling would survive on
        // (`PlacementUtils.filteredByBlockSurvival`).
        if !matches!(
            canvas.get(x, ocean_floor - 1, z),
            Block::GrassBlock | Block::Dirt | Block::CoarseDirt | Block::Podzol | Block::Mycelium
        ) {
            continue;
        }

        // `RandomSelectorFeature`: try each weighted species in order, then fall back.
        let mut config = recipe.default;
        for species in recipe.weighted {
            if random.next_float() < species.chance {
                config = species.config;
                break;
            }
        }
        tree::place_tree(canvas, random, x, ocean_floor, z, &config);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `countExtra` must be one draw, and must produce `count + extra` on exactly one weight in
    /// `1/chance`. Getting the rare branch backwards would make plains denser than forest.
    #[test]
    fn count_extra_is_rare_on_the_bonus_branch() {
        let frequency = CountExtra::new(0, 20, 1);
        let mut random = WorldgenRandom::from_seed(99);
        let mut bonus = 0;
        let trials = 20_000;
        for _ in 0..trials {
            if frequency.sample(&mut random) == 1 {
                bonus += 1;
            }
        }
        let rate = bonus as f64 / trials as f64;
        assert!(
            (rate - 0.05).abs() < 0.01,
            "countExtra(0, 0.05, 1) produced the bonus {rate:.3} of the time, expected ~0.05"
        );
    }

    /// The biome→tree table must agree with the tint table's biome names.
    ///
    /// Both key off `mc::biome`'s registry strings, and a typo in either silently means "this
    /// biome grows nothing" — which looks exactly like a correctly bare biome.
    #[test]
    fn every_tree_biome_is_a_real_biome() {
        let known: Vec<&'static str> = crate::mc::biome::overworld_biomes()
            .into_iter()
            .map(|(_, name)| name)
            .collect();
        for biome in [
            "plains", "sunflower_plains", "meadow", "forest", "flower_forest", "birch_forest",
            "old_growth_birch_forest", "taiga", "old_growth_pine_taiga", "old_growth_spruce_taiga",
            "windswept_hills", "windswept_gravelly_hills", "windswept_forest", "savanna",
            "savanna_plateau", "windswept_savanna", "snowy_plains", "snowy_taiga", "snowy_slopes",
            "ice_spikes", "grove",
        ] {
            assert!(
                known.contains(&biome),
                "{biome} is in the tree table but is not a biome the generator emits"
            );
            assert!(
                tree_kind_for_biome(biome).is_some(),
                "{biome} should map to a tree kind"
            );
        }
    }

    /// Oceans and deserts must stay bare — the most visible way for this to go wrong.
    #[test]
    fn biomes_without_trees_get_none() {
        for biome in ["ocean", "deep_ocean", "warm_ocean", "desert", "river", "beach"] {
            assert_eq!(tree_kind_for_biome(biome), None, "{biome} must not grow trees");
        }
    }

    /// Every kind must have a distinct feature index — a collision would make two biomes' trees
    /// share a random stream and land on top of each other.
    #[test]
    fn feature_indices_are_distinct() {
        let mut seen = std::collections::BTreeSet::new();
        for kind in TreeKind::ALL {
            assert!(
                seen.insert(kind.feature_index()),
                "{kind:?} reuses a feature index"
            );
        }
    }
}
