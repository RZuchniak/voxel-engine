//! Greedy-mesher behaviour locked down as exact counts.
//!
//! These guard the optimizations planned for the mesher — removing the per-mesh 5-chunk
//! deep clone, and any vertex-format change. A refactor that is supposed to be
//! behaviour-preserving must not move these numbers.

use std::collections::BTreeSet;

use voxel_engine::{
    biome_tint::{self, TintKind},
    block::BlockId,
    mesh::{mesh_chunk, AO_MASK, FACE_SHADE_SHIFT, TINT_MASK, TINT_SHIFT},
    world::{Chunk, World, BIOME_CELLS, SECTION_SIZE},
};

/// Fill one 16x16x16 section (world y 0..15) of `coord` with `block`.
fn solid_chunk(coord: (i32, i32), block: BlockId) -> Chunk {
    let mut chunk = Chunk::new(coord);
    for z in 0..SECTION_SIZE {
        for x in 0..SECTION_SIZE {
            for y in 0..SECTION_SIZE as i32 {
                chunk.set_block_world(x, y, z, block);
            }
        }
    }
    chunk
}

fn quad_count(world: &World, coord: (i32, i32)) -> usize {
    mesh_chunk(world, coord)
        .iter()
        .map(|(_, mesh)| mesh.indices().len() / 6)
        .sum()
}

#[test]
fn isolated_solid_section_merges_to_six_quads() {
    let mut world = World::new();
    world.insert_chunk(solid_chunk((0, 0), BlockId::STONE));

    let meshes = mesh_chunk(&world, (0, 0));
    assert_eq!(meshes.len(), 1, "only one section is populated");

    let (_, mesh) = &meshes[0];
    // A solid cube with air on all sides: greedy meshing collapses each face to a
    // single 16x16 quad, so 6 quads -> 24 vertices, 36 indices.
    assert_eq!(mesh.indices().len() / 6, 6, "expected one quad per face");
    assert_eq!(mesh.vertices().len(), 24);
    assert_eq!(mesh.indices().len(), 36);
}

#[test]
fn neighbour_presence_culls_the_shared_face() {
    let mut alone = World::new();
    alone.insert_chunk(solid_chunk((0, 0), BlockId::STONE));
    let quads_alone = quad_count(&alone, (0, 0));

    let mut with_neighbour = World::new();
    with_neighbour.insert_chunk(solid_chunk((0, 0), BlockId::STONE));
    with_neighbour.insert_chunk(solid_chunk((1, 0), BlockId::STONE));
    let quads_with_neighbour = quad_count(&with_neighbour, (0, 0));

    // The +X face is now buried, so exactly one quad should disappear. This is the
    // property that makes the neighbour snapshot necessary at all — if a refactor drops
    // neighbour data, this test fails.
    assert_eq!(quads_alone, 6);
    assert_eq!(quads_with_neighbour, 5, "shared face should be culled");
}

#[test]
fn air_chunk_produces_no_mesh() {
    let mut world = World::new();
    world.insert_chunk(Chunk::new((0, 0)));
    assert!(mesh_chunk(&world, (0, 0)).is_empty());
}

#[test]
fn missing_chunk_produces_no_mesh() {
    let world = World::new();
    assert!(mesh_chunk(&world, (42, -7)).is_empty());
}

#[test]
fn transparent_neighbour_does_not_cull() {
    // Water is not opaque, so a stone face against it must still be emitted.
    let mut world = World::new();
    world.insert_chunk(solid_chunk((0, 0), BlockId::STONE));
    world.insert_chunk(solid_chunk((1, 0), BlockId::WATER));
    assert_eq!(
        quad_count(&world, (0, 0)),
        6,
        "a non-opaque neighbour must not cull the shared face"
    );
}

/// Opaque geometry must land exactly on the block lattice, so faces meet at their shared edges.
///
/// This used to fail: every quad was pushed 0.002 blocks out along its own normal. Two
/// perpendicular faces of the same block then both moved away from the edge between them, so at
/// every **convex** edge the two planes missed each other and left a 0.002-wide slot running the
/// length of the edge. You see straight through it, and against the sky clear colour that reads
/// as a bright hairline outlining every block — visible natively and in the browser.
///
/// Off-lattice positions are the whole signature, so that is what this checks. Faces that share
/// an edge with bit-identical vertices rasterize watertight, which is why no offset is needed.
#[test]
fn opaque_faces_land_on_the_block_lattice() {
    let mut world = World::new();
    world.insert_chunk(solid_chunk((0, 0), BlockId::STONE));

    let meshes = mesh_chunk(&world, (0, 0));
    assert!(!meshes.is_empty(), "the solid section must produce geometry");
    for (_, mesh) in &meshes {
        for vertex in mesh.vertices() {
            for (axis, value) in vertex.position.iter().enumerate() {
                assert_eq!(
                    *value,
                    value.round(),
                    "axis {axis} of {:?} is off the lattice by {}",
                    vertex.position,
                    (value - value.round()).abs()
                );
            }
        }
    }
}

/// Every face must carry the directional brightness index matching its own normal.
///
/// Vanilla multiplies each face by a constant that depends only on its facing — up 1.0, down 0.5,
/// north/south 0.8, east/west 0.6. The engine previously had **no** directional term: its only
/// shading input was a per-vertex occluder count, which varies with position rather than facing,
/// so every face of an isolated cube came out identically lit and the geometry read flat.
///
/// The split between this test and the shader matters: the *index* is a mesher fact and is checked
/// here; the four *factors* live in `square.wgsl` and are not reachable from CPU code. So this
/// catches a mis-packed or mis-assigned face, which is the failure that would silently ruin the
/// lighting, but it cannot catch someone editing 0.5 to 0.9 in the shader.
#[test]
fn every_face_carries_its_own_directional_brightness() {
    let mut world = World::new();
    world.insert_chunk(solid_chunk((0, 0), BlockId::STONE));

    let meshes = mesh_chunk(&world, (0, 0));
    let (_, mesh) = &meshes[0];
    let vertices = mesh.vertices();
    assert_eq!(vertices.len(), 24, "one 16x16 quad per face, 4 vertices each");

    let mut seen = std::collections::BTreeSet::new();
    for quad in vertices.chunks_exact(4) {
        // A greedy quad is axis-aligned, so exactly one axis is constant across its corners —
        // that axis is the face normal.
        let normal_axis = (0..3)
            .find(|&axis| quad.iter().all(|v| v.position[axis] == quad[0].position[axis]))
            .expect("a greedy quad must be axis-aligned");
        let expected = match normal_axis {
            1 if quad[0].position[1] == SECTION_SIZE as f32 => 0, // up
            1 => 1,                                              // down
            2 => 2,                                              // north/south
            _ => 3,                                              // east/west
        };

        let index = (quad[0].light >> FACE_SHADE_SHIFT) & 3;
        assert_eq!(
            index, expected,
            "face with normal axis {normal_axis} at {:?} got brightness index {index}",
            quad[0].position
        );
        for v in quad {
            // The shader reads this flat, so a quad whose corners disagree would take an
            // arbitrary one of them.
            assert_eq!((v.light >> FACE_SHADE_SHIFT) & 3, index, "corners must agree");
            // The packing must not have eaten the occlusion term sharing the same word.
            assert!(v.light & AO_MASK > 0, "AO must survive the packing");
        }
        seen.insert(expected);
    }
    assert_eq!(
        seen,
        BTreeSet::from([0, 1, 2, 3]),
        "all four of vanilla's brightness classes must appear on a lone cube"
    );
}

/// Leaves must not delete the face of a solid block they touch.
///
/// Reported as "where leaves touch solid blocks you can see through the world". Leaves are a cutout
/// — binary alpha, ~60% covered — but were declared `is_opaque`, so the mesher culled the
/// neighbouring ground's face and the leaf's own holes then looked straight at the sky. The second
/// half of the same bug is that only non-opaque full cubes are emitted double-sided, so backface
/// culling removed the far side of even a lone leaf cube; `isolated_solid_section_merges_to_six_quads`
/// covers the single-block shape, and this covers the interaction.
#[test]
fn leaves_do_not_cull_the_face_of_an_adjacent_solid_block() {
    let mut world = World::new();
    world.insert_chunk(solid_chunk((0, 0), BlockId::STONE));
    world.insert_chunk(solid_chunk((1, 0), BlockId::OAK_LEAVES));
    let with_leaves = quad_count(&world, (0, 0));

    // The same stone chunk with nothing beside it: the shared face must survive in both cases.
    let mut alone = World::new();
    alone.insert_chunk(solid_chunk((0, 0), BlockId::STONE));
    let isolated = quad_count(&alone, (0, 0));

    assert_eq!(
        with_leaves, isolated,
        "leaves next to stone removed {} of the stone's quads — a cutout neighbour must not cull",
        isolated as i64 - with_leaves as i64
    );

    // And an opaque neighbour still *must* cull, or this "fix" would just disable face culling.
    let mut opaque = World::new();
    opaque.insert_chunk(solid_chunk((0, 0), BlockId::STONE));
    opaque.insert_chunk(solid_chunk((1, 0), BlockId::DIRT));
    assert!(
        quad_count(&opaque, (0, 0)) < isolated,
        "an opaque neighbour must still cull the shared face"
    );
}

/// A boundary between two different see-through full cubes must carry exactly one face.
///
/// Reported as water and ice "fighting" where they touch, resolving in the water's favour as you
/// approach — the signature of z-fighting. Both were non-opaque full cubes, so neither culled the
/// other, and both are emitted double-sided: four quads on one plane, two of them visible.
///
/// Ice now occludes (`info_solid_looking`), which handles water↔ice the way vanilla does — the ice
/// face survives and the water's is culled. The `block.0 > neighbour.0` tie-break in the mesher
/// covers the pairs where *neither* occludes, such as water↔lava.
#[test]
fn two_see_through_blocks_share_exactly_one_face() {
    for (a, b) in [
        (BlockId::WATER, BlockId::ICE),
        (BlockId::WATER, BlockId::LAVA),
        (BlockId::OAK_LEAVES, BlockId::WATER),
    ] {
        let mut world = World::new();
        world.insert_chunk(solid_chunk((0, 0), a));
        world.insert_chunk(solid_chunk((1, 0), b));

        // The shared plane is x = 16. Count front faces there from either chunk; the double-sided
        // reversed copies sit 0.002 inside their own block, so they are not on the plane itself.
        let mut on_plane = 0usize;
        for coord in [(0, 0), (1, 0)] {
            for (_, mesh) in mesh_chunk(&world, coord) {
                for quad in mesh.vertices().chunks_exact(4) {
                    if quad.iter().all(|v| v.position[0] == SECTION_SIZE as f32) {
                        on_plane += 1;
                    }
                }
            }
        }
        let a_name = a.info().name;
        let b_name = b.info().name;
        assert_eq!(
            on_plane, 1,
            "{a_name} against {b_name} put {on_plane} coplanar faces on x=16; \
             two of them is the z-fighting that was reported"
        );
    }
}

/// An opaque block beside a see-through one must keep its face — the tie-break must not reach it.
#[test]
fn the_see_through_tie_break_never_culls_an_opaque_face() {
    let mut world = World::new();
    world.insert_chunk(solid_chunk((0, 0), BlockId::STONE));
    world.insert_chunk(solid_chunk((1, 0), BlockId::WATER));

    let mut on_plane = 0usize;
    for (_, mesh) in mesh_chunk(&world, (0, 0)) {
        for quad in mesh.vertices().chunks_exact(4) {
            if quad.iter().all(|v| v.position[0] == SECTION_SIZE as f32) {
                on_plane += 1;
            }
        }
    }
    // Stone's id is lower than water's, so an id-only rule would happen to keep this face; the
    // point is that stone occludes, so the tie-break must not consider it at all.
    assert_eq!(on_plane, 1, "stone's face against water must survive");
    assert!(
        BlockId::STONE.occludes_faces() && !BlockId::WATER.occludes_faces(),
        "the premise of this test"
    );
}

/// A fluid's reversed copy is the one face allowed off the lattice — inward, never outward.
///
/// Outward is what tore the convex edges open (see above). Inward only moves the face towards a
/// viewer who is already inside the fluid, so it cannot open a seam on the outside silhouette.
#[test]
fn a_fluid_back_face_is_inset_never_expanded() {
    let mut world = World::new();
    world.insert_chunk(solid_chunk((0, 0), BlockId::WATER));

    let meshes = mesh_chunk(&world, (0, 0));
    assert!(!meshes.is_empty(), "the water section must produce geometry");
    // The section spans world y 0..15, so the top face sits at y = 16 and the bottom at y = 0.
    // Every vertex must be inside or on that slab: nothing may poke out past the block bounds.
    for (_, mesh) in &meshes {
        for vertex in mesh.vertices() {
            let [x, y, z] = vertex.position;
            assert!(
                (0.0..=16.0).contains(&x)
                    && (0.0..=16.0).contains(&y)
                    && (0.0..=16.0).contains(&z),
                "{:?} lies outside the block volume — a face was expanded outward",
                vertex.position
            );
        }
    }
}

/// The mesher must draw every column's surface, whatever the terrain does inside one chunk.
///
/// This used to fail: the mesher kept only sections within a fixed depth of the chunk's
/// *highest* block, so on a slope the columns that started lower fell out of that band and
/// their ground was never meshed — scattered holes across steep hillsides, far worse in the
/// browser (24-block band) than natively (48). The band is gone; this pins the property that
/// replaced it.
#[test]
fn steep_chunk_meshes_every_column_top() {
    // A ramp across the chunk: 40 blocks of relief, deeper than any band ever was.
    let mut chunk = Chunk::new((0, 0));
    let mut tops = [0i32; SECTION_SIZE * SECTION_SIZE];
    for z in 0..SECTION_SIZE {
        for x in 0..SECTION_SIZE {
            let top = 60 + (x as i32 + z as i32) * 40 / 30;
            tops[z * SECTION_SIZE + x] = top;
            for y in 20..=top {
                chunk.set_block_world(x, y, z, BlockId::STONE);
            }
        }
    }

    let mut world = World::new();
    world.insert_chunk(chunk);
    let meshed: Vec<usize> = mesh_chunk(&world, (0, 0))
        .into_iter()
        .map(|(idx, _)| idx)
        .collect();
    for &top in &tops {
        let section_index = (top.div_euclid(SECTION_SIZE as i32) + 4) as usize;
        assert!(
            meshed.contains(&section_index),
            "section {section_index} holding a column top at y={top} was not meshed"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Biome tint
//
// Grass, foliage and water take their colour from the biome, delivered as an 8-bit palette index
// packed into bits 10..17 of the vertex `light` word. Everything below guards a property that is
// invisible in a screenshot until it is wrong across a whole biome.
// ---------------------------------------------------------------------------------------------

/// The tint index a vertex carries.
fn tint_of(vertex: &voxel_engine::Vertex) -> u8 {
    ((vertex.light >> TINT_SHIFT) & TINT_MASK) as u8
}

/// Every distinct tint index in a chunk's mesh.
fn tints_in(world: &World, coord: (i32, i32)) -> BTreeSet<u8> {
    mesh_chunk(world, coord)
        .iter()
        .flat_map(|(_, mesh)| mesh.vertices().iter().map(tint_of))
        .collect()
}

/// A grass block in a swamp must not be painted with plains' green.
///
/// This is the whole feature in one assertion: the biome reaches the mesh, and two different
/// biomes produce two different colours. Before tinting existed both came out identical, which
/// is why a swamp, a jungle and a savanna were indistinguishable.
#[test]
fn grass_takes_its_colour_from_the_biome() {
    let tint_for = |biome: &str| -> BTreeSet<u8> {
        let mut chunk = solid_chunk((0, 0), BlockId::GRASS);
        chunk.set_biomes([biome_tint::biome_index(biome); BIOME_CELLS]);
        let mut world = World::new();
        world.insert_chunk(chunk);
        tints_in(&world, (0, 0))
    };

    let swamp = tint_for("swamp");
    let plains = tint_for("plains");

    let expected_swamp =
        biome_tint::palette_index(biome_tint::biome_index("swamp"), TintKind::Grass);
    assert!(
        swamp.contains(&expected_swamp),
        "swamp grass carries {swamp:?}, expected to include {expected_swamp}"
    );
    assert_ne!(
        swamp, plains,
        "swamp and plains grass produced the same tint — the biome is not reaching the mesher"
    );
    // Exactly two tints, and which two is the point: the top and the four sides take the biome's
    // grass colour, while the **bottom** is plain dirt and must stay neutral. Tinting a grass
    // block wholesale would turn its underside green, which is visible from any overhang.
    assert_eq!(
        swamp,
        BTreeSet::from([biome_tint::NEUTRAL, expected_swamp]),
        "a grass block should carry its biome's grass tint on top and sides, and neutral on its \
         dirt underside"
    );
}

/// Stone must be biome-independent — and this is a performance property, not just a visual one.
///
/// If an untinted block's palette index varied with biome, every greedy run of stone would break
/// at each 4-block biome cell boundary. Underground is the overwhelming majority of the world's
/// geometry, so that would be a large and completely invisible cost.
#[test]
fn untinted_blocks_are_neutral_whatever_the_biome() {
    for biome in ["swamp", "jungle", "badlands", "warm_ocean"] {
        let mut chunk = solid_chunk((0, 0), BlockId::STONE);
        chunk.set_biomes([biome_tint::biome_index(biome); BIOME_CELLS]);
        let mut world = World::new();
        world.insert_chunk(chunk);
        assert_eq!(
            tints_in(&world, (0, 0)),
            BTreeSet::from([biome_tint::NEUTRAL]),
            "stone in {biome} is not neutral — greedy runs will break at biome boundaries"
        );
    }
}

/// A chunk from a source with no biome data must still get a colour — not white.
///
/// The Anvil zip-import path supplies no biomes, and this is the one place where "fall back to
/// neutral" is the *wrong* answer. Tintable texture layers are neutralised to grayscale at load
/// on the assumption that a biome colours them, so an untinted grass block renders **grey**.
/// Imported worlds would have come out with grey grass, grey leaves and grey water — a
/// regression on a headline feature, and one that no procedurally-generated test would catch
/// because those chunks always carry biomes.
#[test]
fn a_chunk_without_biome_data_falls_back_to_the_default_biome() {
    let mut world = World::new();
    world.insert_chunk(solid_chunk((0, 0), BlockId::GRASS));

    let expected = biome_tint::palette_index(biome_tint::DEFAULT_BIOME, TintKind::Grass);
    let tints = tints_in(&world, (0, 0));
    assert!(
        tints.contains(&expected),
        "a biome-less chunk's grass carries {tints:?}, expected the default biome's {expected}"
    );
    assert_ne!(
        tints,
        BTreeSet::from([biome_tint::NEUTRAL]),
        "grass with no biome data must not render untinted — the texture is grayscale, so \
         neutral means grey grass"
    );
}

/// A biome boundary inside one chunk must split the greedy quad that crosses it.
///
/// Merging across it would paint one biome's grass with its neighbour's colour in a straight
/// line along the merge axis — subtle enough to survive a screenshot, so it needs a test.
#[test]
fn a_biome_boundary_breaks_a_greedy_run() {
    let mut uniform = solid_chunk((0, 0), BlockId::GRASS);
    uniform.set_biomes([biome_tint::biome_index("plains"); BIOME_CELLS]);

    // Split the 4x4 biome grid down the middle: plains on one side, swamp on the other.
    let mut split_grid = [biome_tint::biome_index("plains"); BIOME_CELLS];
    for cell in 0..BIOME_CELLS {
        if cell % 4 >= 2 {
            split_grid[cell] = biome_tint::biome_index("swamp");
        }
    }
    let mut split = solid_chunk((0, 0), BlockId::GRASS);
    split.set_biomes(split_grid);

    let quads_of = |chunk: Chunk| {
        let mut world = World::new();
        world.insert_chunk(chunk);
        quad_count(&world, (0, 0))
    };

    let uniform_quads = quads_of(uniform);
    let split_quads = quads_of(split);
    assert!(
        split_quads > uniform_quads,
        "a chunk spanning two biomes produced {split_quads} quads, no more than the {uniform_quads} \
         of a single-biome chunk — the tint is not part of the merge key"
    );
}

/// The shader hardcodes the palette length; a mismatch is a GPU-side error or silent miscolouring.
///
/// WGSL cannot import a Rust constant, so the two are pinned to each other here rather than
/// discovered at run time in a browser.
#[test]
fn the_shader_agrees_with_the_rust_palette_length() {
    let shader = include_str!("../src/square.wgsl");
    let declared = shader
        .lines()
        .find_map(|line| {
            let rest = line.trim().strip_prefix("const PALETTE_LEN: u32 = ")?;
            rest.trim_end_matches(';').trim_end_matches('u').parse::<usize>().ok()
        })
        .expect("square.wgsl must declare `const PALETTE_LEN: u32 = <n>u;`");
    assert_eq!(
        declared,
        biome_tint::PALETTE_LEN,
        "square.wgsl says {declared} palette entries, biome_tint says {}",
        biome_tint::PALETTE_LEN
    );
}
