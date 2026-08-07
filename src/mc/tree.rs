//! `TreeFeature` and its placers — the shape of a tree.
//!
//! A port of 26.2 `TreeFeature.java`, `TrunkPlacer`/`StraightTrunkPlacer`,
//! `FoliagePlacer`/`BlobFoliagePlacer`/`SpruceFoliagePlacer`/`PineFoliagePlacer` and
//! `TwoLayersFeatureSize`. Everything here consumes [`WorldgenRandom`] in exactly the order
//! Minecraft does — the draw *order* is as load-bearing as the arithmetic, because a single
//! extra or missing draw shifts every tree placed after it in the same chunk.
//!
//! What this module deliberately does **not** cover:
//!
//! - **Tree decorators** (beehives, cocoa, vines, leaf litter). They draw from the same random
//!   after the tree is built, so omitting them is only safe because they are the *last* thing a
//!   tree does and each tree re-seeds from its own placement draw.
//! - **`updateLeaves`**, the BFS that writes the `distance` blockstate property onto leaves.
//!   That property drives decay and rendering-side culling in the real game; this engine has no
//!   blockstate properties, so it is skipped. It places no blocks, so geometry is unaffected.
//! - **Root placers** (mangrove) and the fancy/dark-oak/mega trunk placers.
//!
//! The species covered are the ones that dominate the overworld visually: oak and birch (blob
//! foliage on a straight trunk) and spruce/pine (conical foliage on a straight trunk).

use super::surface::Block;
use super::wgrandom::WorldgenRandom;

/// `IntProvider` — a constant or a uniform range, sampled from the worldgen random.
///
/// Only these two forms appear in the tree configurations ported here. The distinction matters
/// for draw accounting: [`IntProvider::Constant`] consumes **no** randomness, while
/// [`IntProvider::Uniform`] consumes one `nextInt` — so replacing one with the other desyncs
/// everything downstream even when the value happens to match.
#[derive(Debug, Clone, Copy)]
pub enum IntProvider {
    Constant(i32),
    /// `UniformInt.of(min, max)` — **inclusive** on both ends.
    Uniform { min: i32, max: i32 },
}

impl IntProvider {
    #[inline]
    pub fn sample(self, random: &mut WorldgenRandom) -> i32 {
        match self {
            IntProvider::Constant(value) => value,
            // `UniformInt.sample` is `min + random.nextInt(max - min + 1)`.
            IntProvider::Uniform { min, max } => min + random.next_int_bound(max - min + 1),
        }
    }
}

/// `TwoLayersFeatureSize` — how wide the clearance check is at a given height.
#[derive(Debug, Clone, Copy)]
pub struct FeatureSize {
    pub limit: i32,
    pub lower_size: i32,
    pub upper_size: i32,
    /// `minClippedHeight`: if set, a tree whose clearance is cut short may still be placed as
    /// long as it reaches this height.
    pub min_clipped_height: Option<i32>,
}

impl FeatureSize {
    pub const fn two_layers(limit: i32, lower_size: i32, upper_size: i32) -> Self {
        Self { limit, lower_size, upper_size, min_clipped_height: None }
    }

    #[inline]
    fn size_at_height(&self, y_offset: i32) -> i32 {
        if y_offset < self.limit { self.lower_size } else { self.upper_size }
    }
}

/// `TrunkPlacer` — only `StraightTrunkPlacer` is ported.
#[derive(Debug, Clone, Copy)]
pub struct TrunkPlacer {
    pub base_height: i32,
    pub height_rand_a: i32,
    pub height_rand_b: i32,
}

impl TrunkPlacer {
    /// `TrunkPlacer.getTreeHeight` — **two** `nextInt` draws, always, even when a bound is zero
    /// (`nextInt(1)` still consumes a draw). Collapsing them into one is the kind of "obvious"
    /// simplification that silently changes every tree.
    fn tree_height(&self, random: &mut WorldgenRandom) -> i32 {
        self.base_height
            + random.next_int_bound(self.height_rand_a + 1)
            + random.next_int_bound(self.height_rand_b + 1)
    }
}

/// `FoliagePlacer` — the three shapes that cover oak/birch and spruce/pine.
#[derive(Debug, Clone, Copy)]
pub enum FoliagePlacer {
    /// `BlobFoliagePlacer` — oak, birch, jungle.
    Blob { radius: IntProvider, offset: IntProvider, height: i32 },
    /// `SpruceFoliagePlacer` — the ragged conifer cone.
    Spruce { radius: IntProvider, offset: IntProvider, trunk_height: IntProvider },
    /// `PineFoliagePlacer` — a narrow cone with a bare trunk below it.
    Pine { radius: IntProvider, offset: IntProvider, height: IntProvider },
}

impl FoliagePlacer {
    fn radius_provider(&self) -> IntProvider {
        match *self {
            FoliagePlacer::Blob { radius, .. }
            | FoliagePlacer::Spruce { radius, .. }
            | FoliagePlacer::Pine { radius, .. } => radius,
        }
    }

    fn offset_provider(&self) -> IntProvider {
        match *self {
            FoliagePlacer::Blob { offset, .. }
            | FoliagePlacer::Spruce { offset, .. }
            | FoliagePlacer::Pine { offset, .. } => offset,
        }
    }

    /// `FoliagePlacer.foliageHeight`.
    fn foliage_height(&self, random: &mut WorldgenRandom, tree_height: i32) -> i32 {
        match *self {
            FoliagePlacer::Blob { height, .. } => height,
            FoliagePlacer::Spruce { trunk_height, .. } => {
                (tree_height - trunk_height.sample(random)).max(4)
            }
            FoliagePlacer::Pine { height, .. } => height.sample(random),
        }
    }

    /// `FoliagePlacer.foliageRadius`. Pine adds an extra draw on top of the base radius.
    fn foliage_radius(&self, random: &mut WorldgenRandom, trunk_height: i32) -> i32 {
        let base = self.radius_provider().sample(random);
        match self {
            FoliagePlacer::Pine { .. } => base + random.next_int_bound((trunk_height + 1).max(1)),
            _ => base,
        }
    }

    /// `FoliagePlacer.shouldSkipLocation` — which corner leaves get knocked out.
    ///
    /// Blob's variant consumes randomness (`random.nextInt(2) == 0`) and the others do not, so
    /// this is another place where the draw count is part of the shape.
    #[inline]
    fn should_skip(
        &self,
        random: &mut WorldgenRandom,
        dx: i32,
        y: i32,
        dz: i32,
        current_radius: i32,
    ) -> bool {
        match self {
            FoliagePlacer::Blob { .. } => {
                dx == current_radius
                    && dz == current_radius
                    && (random.next_int_bound(2) == 0 || y == 0)
            }
            FoliagePlacer::Spruce { .. } | FoliagePlacer::Pine { .. } => {
                dx == current_radius && dz == current_radius && current_radius > 0
            }
        }
    }
}

/// A `TreeConfiguration` — one species' full recipe.
#[derive(Debug, Clone, Copy)]
pub struct TreeConfig {
    pub log: Block,
    pub leaves: Block,
    pub trunk: TrunkPlacer,
    pub foliage: FoliagePlacer,
    pub size: FeatureSize,
}

/// `TreeFeatures.createStraightBlobTree(OAK_LOG, OAK_LEAVES, 4, 2, 0, 2, ...)`.
pub const OAK: TreeConfig = TreeConfig {
    log: Block::OakLog,
    leaves: Block::OakLeaves,
    trunk: TrunkPlacer { base_height: 4, height_rand_a: 2, height_rand_b: 0 },
    foliage: FoliagePlacer::Blob {
        radius: IntProvider::Constant(2),
        offset: IntProvider::Constant(0),
        height: 3,
    },
    size: FeatureSize::two_layers(1, 0, 1),
};

/// `TreeFeatures.createBirch` — same shape as oak, one block taller at the base.
pub const BIRCH: TreeConfig = TreeConfig {
    log: Block::BirchLog,
    leaves: Block::BirchLeaves,
    trunk: TrunkPlacer { base_height: 5, height_rand_a: 2, height_rand_b: 0 },
    foliage: FoliagePlacer::Blob {
        radius: IntProvider::Constant(2),
        offset: IntProvider::Constant(0),
        height: 3,
    },
    size: FeatureSize::two_layers(1, 0, 1),
};

/// `TreeFeatures.SPRUCE`.
pub const SPRUCE: TreeConfig = TreeConfig {
    log: Block::SpruceLog,
    leaves: Block::SpruceLeaves,
    trunk: TrunkPlacer { base_height: 5, height_rand_a: 2, height_rand_b: 1 },
    foliage: FoliagePlacer::Spruce {
        radius: IntProvider::Uniform { min: 2, max: 3 },
        offset: IntProvider::Uniform { min: 0, max: 2 },
        trunk_height: IntProvider::Uniform { min: 1, max: 2 },
    },
    size: FeatureSize::two_layers(2, 0, 2),
};

/// `TreeFeatures.PINE` — spruce logs, a bare trunk and a narrow crown.
pub const PINE: TreeConfig = TreeConfig {
    log: Block::SpruceLog,
    leaves: Block::SpruceLeaves,
    trunk: TrunkPlacer { base_height: 6, height_rand_a: 4, height_rand_b: 0 },
    foliage: FoliagePlacer::Pine {
        radius: IntProvider::Constant(1),
        offset: IntProvider::Constant(1),
        height: IntProvider::Uniform { min: 3, max: 4 },
    },
    size: FeatureSize::two_layers(2, 0, 2),
};

/// Somewhere a tree can read and write blocks, spanning more than one chunk.
///
/// Trees are placed with their trunk inside a chunk but their foliage routinely crosses into the
/// neighbour, so the feature cannot be confined to a 16×16 column — see `mc::decorate` for the
/// 3×3 canvas this is implemented over.
pub trait TreeCanvas {
    fn get(&self, x: i32, y: i32, z: i32) -> Block;
    fn set(&mut self, x: i32, y: i32, z: i32, block: Block);
    /// Inclusive world-Y bounds.
    fn min_y(&self) -> i32;
    fn max_y(&self) -> i32;
}

/// `TreeFeature.validTreePos` — air or a block trees may replace.
///
/// Vanilla tests `BlockTags.REPLACEABLE_BY_TREES`, which is foliage/snow/plants. This engine
/// generates none of those, so the tag reduces to air plus the fluids a tree may grow through.
#[inline]
fn valid_tree_pos(block: Block) -> bool {
    matches!(block, Block::Air | Block::Water)
}

/// `TrunkPlacer.isFree` — valid position, or an existing log (so trunks may pass through one
/// another rather than aborting).
#[inline]
fn is_free(block: Block) -> bool {
    valid_tree_pos(block) || is_log(block)
}

#[inline]
fn is_log(block: Block) -> bool {
    matches!(block, Block::OakLog | Block::BirchLog | Block::SpruceLog)
}

/// `TreeFeature.getMaxFreeTreeHeight` — how tall the tree can grow before hitting something.
fn max_free_tree_height<C: TreeCanvas + ?Sized>(
    canvas: &C,
    max_tree_height: i32,
    x: i32,
    y: i32,
    z: i32,
    config: &TreeConfig,
) -> i32 {
    for dy in 0..=max_tree_height + 1 {
        let r = config.size.size_at_height(dy);
        for dx in -r..=r {
            for dz in -r..=r {
                if !is_free(canvas.get(x + dx, y + dy, z + dz)) {
                    return dy - 2;
                }
            }
        }
    }
    max_tree_height
}

/// Place one tree with its base at `(x, y, z)`. Returns whether anything was placed.
///
/// The draw order is the contract: tree height, foliage height, foliage radius, then the trunk,
/// then each foliage row. `TreeFeature.doPlace` establishes it and the placers must not reorder
/// their own sampling within it.
///
/// Generic over the canvas so the hot get/set path monomorphises — decoration used to pay
/// `dyn` dispatch on every leaf cell for no semantic reason.
pub fn place_tree<C: TreeCanvas + ?Sized>(
    canvas: &mut C,
    random: &mut WorldgenRandom,
    x: i32,
    y: i32,
    z: i32,
    config: &TreeConfig,
) -> bool {
    let tree_height = config.trunk.tree_height(random);
    let foliage_height = config.foliage.foliage_height(random, tree_height);
    let trunk_height = tree_height - foliage_height;
    let leaf_radius = config.foliage.foliage_radius(random, trunk_height);

    let min_y = y;
    let max_y = y + tree_height + 1;
    if min_y < canvas.min_y() + 1 || max_y > canvas.max_y() + 1 {
        return false;
    }

    let clipped = max_free_tree_height(canvas, tree_height, x, y, z, config);
    let tall_enough = clipped >= tree_height
        || config.size.min_clipped_height.is_some_and(|min| clipped >= min);
    if !tall_enough {
        return false;
    }

    // `StraightTrunkPlacer.placeTrunk`. `placeBelowTrunkBlock` is skipped: `belowTrunkProvider`
    // is dirt in vanilla, and the tree only ever grows on ground that is already dirt or grass.
    for dy in 0..clipped {
        if valid_tree_pos(canvas.get(x, y + dy, z)) {
            canvas.set(x, y + dy, z, config.log);
        }
    }
    // The single `FoliageAttachment` a straight trunk returns, at the top of the trunk.
    let attach_y = y + clipped;

    let offset = config.foliage.offset_provider().sample(random);
    create_foliage(
        canvas,
        random,
        x,
        attach_y,
        z,
        config,
        foliage_height,
        leaf_radius,
        offset,
    );
    true
}

/// The per-shape `createFoliage` bodies.
#[allow(clippy::too_many_arguments)]
fn create_foliage<C: TreeCanvas + ?Sized>(
    canvas: &mut C,
    random: &mut WorldgenRandom,
    x: i32,
    y: i32,
    z: i32,
    config: &TreeConfig,
    foliage_height: i32,
    leaf_radius: i32,
    offset: i32,
) {
    match config.foliage {
        FoliagePlacer::Blob { .. } => {
            let mut yo = offset;
            while yo >= offset - foliage_height {
                // `radiusOffset` is 0 for a straight trunk's single attachment.
                let current_radius = (leaf_radius - 1 - yo / 2).max(0);
                place_leaves_row(canvas, random, x, y, z, config, current_radius, yo);
                yo -= 1;
            }
        }
        FoliagePlacer::Spruce { .. } => {
            let mut current_radius = random.next_int_bound(2);
            let mut max_radius = 1;
            let mut min_radius = 0;
            let mut yo = offset;
            while yo >= -foliage_height {
                place_leaves_row(canvas, random, x, y, z, config, current_radius, yo);
                if current_radius >= max_radius {
                    current_radius = min_radius;
                    min_radius = 1;
                    max_radius = (max_radius + 1).min(leaf_radius);
                } else {
                    current_radius += 1;
                }
                yo -= 1;
            }
        }
        FoliagePlacer::Pine { .. } => {
            let mut current_radius = 0;
            let mut yo = offset;
            while yo >= offset - foliage_height {
                place_leaves_row(canvas, random, x, y, z, config, current_radius, yo);
                if current_radius >= 1 && yo == offset - foliage_height + 1 {
                    current_radius -= 1;
                } else if current_radius < leaf_radius {
                    current_radius += 1;
                }
                yo -= 1;
            }
        }
    }
}

/// `FoliagePlacer.placeLeavesRow` — one square ring of leaves, corners possibly knocked out.
#[allow(clippy::too_many_arguments)]
fn place_leaves_row<C: TreeCanvas + ?Sized>(
    canvas: &mut C,
    random: &mut WorldgenRandom,
    x: i32,
    y: i32,
    z: i32,
    config: &TreeConfig,
    current_radius: i32,
    y_offset: i32,
) {
    for dx in -current_radius..=current_radius {
        for dz in -current_radius..=current_radius {
            // `shouldSkipLocationSigned` folds the signed offsets to absolute values first, and
            // it is called for **every** cell — including ones that will not be skipped — so its
            // random draws happen regardless of the outcome.
            if !config.foliage.should_skip(random, dx.abs(), y_offset, dz.abs(), current_radius) {
                let (px, py, pz) = (x + dx, y + y_offset, z + dz);
                // `tryPlaceLeaf`: only into air/water, never over a log.
                if valid_tree_pos(canvas.get(px, py, pz)) {
                    canvas.set(px, py, pz, config.leaves);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small fixed-size canvas for testing one tree in isolation.
    struct TestCanvas {
        blocks: Vec<Block>,
        size: i32,
        height: i32,
    }

    impl TestCanvas {
        fn new(size: i32, height: i32) -> Self {
            Self { blocks: vec![Block::Air; (size * size * height) as usize], size, height }
        }
        fn index(&self, x: i32, y: i32, z: i32) -> Option<usize> {
            if x < 0 || z < 0 || y < 0 || x >= self.size || z >= self.size || y >= self.height {
                return None;
            }
            Some(((y * self.size + z) * self.size + x) as usize)
        }
        fn count(&self, block: Block) -> usize {
            self.blocks.iter().filter(|b| **b == block).count()
        }
    }

    impl TreeCanvas for TestCanvas {
        fn get(&self, x: i32, y: i32, z: i32) -> Block {
            self.index(x, y, z).map_or(Block::Stone, |i| self.blocks[i])
        }
        fn set(&mut self, x: i32, y: i32, z: i32, block: Block) {
            if let Some(i) = self.index(x, y, z) {
                self.blocks[i] = block;
            }
        }
        fn min_y(&self) -> i32 {
            0
        }
        fn max_y(&self) -> i32 {
            self.height - 1
        }
    }

    fn grow(config: &TreeConfig, seed: i64) -> TestCanvas {
        let mut canvas = TestCanvas::new(32, 64);
        let mut random = WorldgenRandom::from_seed(seed);
        assert!(
            place_tree(&mut canvas, &mut random, 16, 8, 16, config),
            "the tree should have had room to grow"
        );
        canvas
    }

    /// An oak must produce a connected trunk and a crown above it.
    #[test]
    fn an_oak_has_a_trunk_and_a_crown() {
        let canvas = grow(&OAK, 42);
        let logs = canvas.count(Block::OakLog);
        let leaves = canvas.count(Block::OakLeaves);

        // `StraightTrunkPlacer(4, 2, 0)` → height 4..=6.
        assert!((4..=6).contains(&logs), "oak trunk was {logs} logs, expected 4..=6");
        assert!(leaves > 20, "oak crown was only {leaves} leaves");

        // The trunk must be a contiguous vertical run starting at the base.
        for dy in 0..logs as i32 {
            assert_eq!(
                canvas.get(16, 8 + dy, 16),
                Block::OakLog,
                "trunk broken at dy={dy}"
            );
        }
        // Leaves must sit at and above the top of the trunk, never below the base.
        assert_eq!(canvas.get(16, 7, 16), Block::Air, "nothing may be placed below the trunk");
    }

    /// Foliage must never overwrite the trunk — `tryPlaceLeaf` only writes into air.
    #[test]
    fn leaves_never_replace_the_trunk() {
        for seed in 0..40i64 {
            let canvas = grow(&OAK, seed);
            let logs = canvas.count(Block::OakLog);
            assert!(logs >= 4, "seed {seed} produced a {logs}-log trunk");
        }
    }

    /// A spruce must be a **taper**, not a blob — that silhouette is the whole point of having a
    /// second foliage placer, and it is what tells a taiga from a forest at a glance.
    ///
    /// Note it is *not* narrower than an oak overall: `SpruceFoliagePlacer` grows its radius
    /// downward to `leaf_radius` (2..=3), so the skirt is **wider** than an oak's blob (max 2).
    /// The distinguishing property is the profile — narrow on top, wide at the bottom — plus
    /// greater height. An earlier version of this test asserted "narrower" and failed against
    /// correct code.
    #[test]
    fn a_spruce_tapers_where_an_oak_is_a_blob() {
        // Aggregated over seeds rather than asserted per tree. `SpruceFoliagePlacer` cycles its
        // radius (`currentRadius` resets to `minRadius` whenever it reaches `maxRadius`), which
        // is what gives a spruce its stacked, ragged rings — so an individual tree's topmost
        // ring is not always strictly narrower than every ring below it. The *shape* is a cone;
        // any single layer is not.
        let mut top_total = 0usize;
        let mut bottom_total = 0usize;
        let mut spruce_taller = 0;
        for seed in 0..25i64 {
            let spruce = grow(&SPRUCE, seed);
            let profile = crown_profile(&spruce);
            top_total += *profile.first().expect("a spruce must have leaves");
            bottom_total += *profile.last().expect("a spruce must have leaves");
            if crown_top(&spruce) > crown_top(&grow(&OAK, seed)) {
                spruce_taller += 1;
            }
        }
        assert!(
            bottom_total > top_total,
            "spruce crowns should widen downward: tips total {top_total}, skirts {bottom_total}"
        );
        assert!(
            spruce_taller >= 20,
            "spruce should out-top oak on most seeds, did so on {spruce_taller}/25"
        );
    }

    /// Width of each leaf layer, from the highest layer downward.
    fn crown_profile(canvas: &TestCanvas) -> Vec<usize> {
        let mut rows = Vec::new();
        for y in (0..canvas.height).rev() {
            let (mut min_x, mut max_x) = (i32::MAX, i32::MIN);
            for z in 0..canvas.size {
                for x in 0..canvas.size {
                    if matches!(
                        canvas.get(x, y, z),
                        Block::OakLeaves | Block::BirchLeaves | Block::SpruceLeaves
                    ) {
                        min_x = min_x.min(x);
                        max_x = max_x.max(x);
                    }
                }
            }
            if min_x <= max_x {
                rows.push((max_x - min_x + 1) as usize);
            } else if !rows.is_empty() {
                break; // past the bottom of the crown
            }
        }
        rows
    }

    /// Highest Y carrying a leaf.
    fn crown_top(canvas: &TestCanvas) -> i32 {
        for y in (0..canvas.height).rev() {
            for z in 0..canvas.size {
                for x in 0..canvas.size {
                    if matches!(
                        canvas.get(x, y, z),
                        Block::OakLeaves | Block::BirchLeaves | Block::SpruceLeaves
                    ) {
                        return y;
                    }
                }
            }
        }
        -1
    }

    /// A tree with no headroom must refuse to place rather than growing into the ceiling.
    #[test]
    fn a_tree_with_no_headroom_is_refused() {
        let mut canvas = TestCanvas::new(32, 64);
        // Cap the column two blocks above the base.
        for z in 0..32 {
            for x in 0..32 {
                canvas.set(x, 10, z, Block::Stone);
            }
        }
        let mut random = WorldgenRandom::from_seed(1);
        assert!(
            !place_tree(&mut canvas, &mut random, 16, 8, 16, &OAK),
            "a tree with 2 blocks of headroom must not be placed"
        );
        assert_eq!(canvas.count(Block::OakLog), 0, "a refused tree must place nothing");
    }

    /// Different seeds must give different trees, and the same seed must repeat exactly.
    #[test]
    fn trees_are_deterministic_but_varied() {
        let a = grow(&OAK, 7);
        let b = grow(&OAK, 7);
        assert_eq!(a.blocks, b.blocks, "the same seed must grow the same tree");

        let mut distinct = std::collections::BTreeSet::new();
        for seed in 0..30i64 {
            distinct.insert(grow(&OAK, seed).count(Block::OakLog));
        }
        assert!(distinct.len() > 1, "every seed produced the same trunk height");
    }
}
