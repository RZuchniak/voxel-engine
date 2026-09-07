//! Trees must survive chunk boundaries.
//!
//! This is the one property the 3×3 decoration pass exists to provide, and the one that no unit
//! test of `mc::tree` can reach: a tree's trunk sits in one chunk and its canopy routinely lands
//! in the next, so a chunk's contents depend on its *neighbours'* decoration. Decorating only the
//! centre chunk and clipping would still produce trees, still pass every geometry test, and leave
//! a sawn-off half canopy on every chunk border.

use voxel_engine::{
    block::BlockId,
    source::{SeededProceduralSource, WorldSource},
    world::{Chunk, SECTION_COUNT, SECTION_SIZE},
};

const SEED: i64 = 6954908675375307936;

fn is_leaf(block: BlockId) -> bool {
    matches!(
        block,
        BlockId::OAK_LEAVES | BlockId::BIRCH_LEAVES | BlockId::SPRUCE_LEAVES
    )
}

fn is_log(block: BlockId) -> bool {
    matches!(
        block,
        BlockId::OAK_LOG | BlockId::BIRCH_LOG | BlockId::SPRUCE_LOG
    )
}

/// Every `(local_x, world_y, local_z)` in the chunk holding a log.
fn logs(chunk: &Chunk) -> Vec<(i32, i32, i32)> {
    positions(chunk, is_log)
}

/// Every `(local_x, world_y, local_z)` in the chunk holding a leaf.
fn leaves(chunk: &Chunk) -> Vec<(i32, i32, i32)> {
    positions(chunk, is_leaf)
}

fn positions(chunk: &Chunk, matches: fn(BlockId) -> bool) -> Vec<(i32, i32, i32)> {
    let mut out = Vec::new();
    for index in 0..SECTION_COUNT {
        let Some(section) = chunk.section(index) else { continue };
        let base_y = (index as i32 - 4) * SECTION_SIZE as i32;
        for ly in 0..SECTION_SIZE {
            for lz in 0..SECTION_SIZE {
                for lx in 0..SECTION_SIZE {
                    if matches(section.block_at(lx, ly, lz)) {
                        out.push((lx as i32, base_y + ly as i32, lz as i32));
                    }
                }
            }
        }
    }
    out
}

/// A canopy touching a chunk's +X edge must continue into the neighbour.
///
/// Searches a small area for a tree that actually straddles a border rather than constructing
/// one, because whether a tree lands on a border is a property of the seed. If the search finds
/// no straddling canopy it fails loudly — a silent skip here would mean the test never ran.
#[test]
fn a_canopy_crossing_a_chunk_border_is_not_cut_off() {
    let source = SeededProceduralSource::new(SEED);

    let mut checked = 0usize;
    for cz in -10..=10 {
        for cx in -10..=10 {
            let chunk = source.load_chunk((cx, cz)).expect("procedural gen cannot fail");
            // Leaves sitting against the +X edge of this chunk.
            let edge: Vec<(i32, i32, i32)> = leaves(&chunk)
                .into_iter()
                .filter(|(lx, _, _)| *lx == SECTION_SIZE as i32 - 1)
                .collect();
            if edge.is_empty() {
                continue;
            }

            // Only trunks **on** the edge column guarantee a crossing. A canopy can legitimately
            // end flush with the border (a trunk at lx=13 with radius 2 spans 11..15 and touches
            // the edge without crossing it), so "has edge leaves" alone proves nothing — an
            // earlier version of this test asserted exactly that and failed against correct code.
            // A blob crown reaches radius 2, so a trunk at lx >= 14 must cross into x >= 16.
            let edge_trunk = logs(&chunk)
                .into_iter()
                .find(|(lx, _, _)| *lx >= SECTION_SIZE as i32 - 2);
            let Some((_, trunk_y, trunk_z)) = edge_trunk else { continue };

            let neighbour = source
                .load_chunk((cx + 1, cz))
                .expect("procedural gen cannot fail");
            // Every blob/spruce/pine crown has radius >= 1, so a trunk in the last column must
            // put leaves in the neighbour's first column somewhere in the crown's height band.
            let crossed = (trunk_y..trunk_y + 12).any(|y| {
                (-2..=2).any(|dz| is_leaf(neighbour.block_at_local(0, y, trunk_z + dz)))
            });
            assert!(
                crossed,
                "a trunk at the +X edge of chunk ({cx},{cz}) (y={trunk_y}, z={trunk_z}) put no \
                 leaves into chunk ({},{cz}) — the canopy was clipped at the border",
                cx + 1
            );
            let _ = edge;
            checked += 1;
            if checked >= 1 {
                return;
            }
        }
    }

    panic!(
        "no canopy touching a chunk border was found in the search area — the test could not \
         check anything, which is not the same as passing"
    );
}

/// Each chunk's trees must run once, even though assembling a column reads a 3×3 of overlays.
///
/// The first `load_chunk` pulls a 5×5 of terrain (each of the 9 overlays needs its own 3×3).
/// Reloading the same coord, and then the neighbour, must reuse those overlays rather than
/// decorate again.
#[test]
fn a_chunk_is_decorated_only_once() {
    let source = SeededProceduralSource::new(SEED);
    let _ = source.load_chunk((0, 0)).expect("gen");
    let (terrain_hits, terrain_misses) = source.cache_stats();
    let (overlay_hits, overlay_misses) = source.overlay_stats();
    assert_eq!(
        overlay_misses, 9,
        "assembling (0,0) should decorate its 3×3 once each, got {overlay_misses} overlay misses"
    );
    assert_eq!(
        terrain_misses, 25,
        "those 9 overlays need a 5×5 of terrain, got {terrain_misses} terrain misses \
         ({terrain_hits} hits)"
    );

    let _ = source.load_chunk((0, 0)).expect("gen");
    let (_, overlay_misses_again) = source.overlay_stats();
    assert_eq!(
        overlay_misses_again, 9,
        "reloading the same chunk must not decorate again, got {overlay_misses_again} overlay misses"
    );
    assert!(
        source.overlay_stats().0 > overlay_hits,
        "reloading should hit the overlay cache"
    );

    let _ = source.load_chunk((1, 0)).expect("gen");
    let (_, overlay_misses_neighbour) = source.overlay_stats();
    let (_, terrain_misses_neighbour) = source.cache_stats();
    assert_eq!(
        overlay_misses_neighbour, 12,
        "the neighbour's 3×3 adds three new overlays, got {overlay_misses_neighbour}"
    );
    assert_eq!(
        terrain_misses_neighbour, 30,
        "the neighbour's 5×5 adds a 5-chunk strip, got {terrain_misses_neighbour}"
    );
}

/// The same chunk must generate identically however many times it is asked for.
///
/// Decoration reads a shared, mutable terrain cache across threads, so this guards the obvious
/// way that could go wrong: a cache that returns a *decorated* chunk where undecorated terrain
/// was expected would grow trees on top of trees, and only on the second request.
#[test]
fn regenerating_a_chunk_gives_the_same_trees() {
    let source = SeededProceduralSource::new(SEED);
    let coord = (2, -3);

    let first = source.load_chunk(coord).expect("procedural gen cannot fail");
    // Pull the neighbours in between, so the cache is exercised and partially evicted.
    for dz in -2..=2 {
        for dx in -2..=2 {
            let _ = source.load_chunk((coord.0 + dx, coord.1 + dz));
        }
    }
    let second = source.load_chunk(coord).expect("procedural gen cannot fail");

    assert_eq!(
        leaves(&first),
        leaves(&second),
        "the same chunk grew different trees on a second request"
    );
}

/// Two sources with the same seed must agree — the property workers rely on.
///
/// On wasm each worker builds its own `SeededProceduralSource` with its own terrain cache, and
/// chunks are never shipped between them. If decoration depended on cache *state* rather than
/// only on the seed, workers would disagree and the seams would show as mismatched trees.
#[test]
fn separate_sources_with_the_same_seed_agree() {
    let a = SeededProceduralSource::new(SEED);
    let b = SeededProceduralSource::new(SEED);

    // Warm one of them along a different path, so the two caches hold different contents.
    for dx in -3..=3 {
        let _ = a.load_chunk((dx, 5));
    }

    for coord in [(0, 0), (1, -1), (-2, 3)] {
        assert_eq!(
            leaves(&a.load_chunk(coord).expect("gen")),
            leaves(&b.load_chunk(coord).expect("gen")),
            "sources with the same seed disagreed at {coord:?}"
        );
    }
}
