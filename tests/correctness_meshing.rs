//! Greedy-mesher behaviour locked down as exact counts.
//!
//! These guard the optimizations planned for the mesher — removing the per-mesh 5-chunk
//! deep clone, and any vertex-format change. A refactor that is supposed to be
//! behaviour-preserving must not move these numbers.

use std::collections::BTreeSet;

use voxel_engine::{
    block::BlockId,
    mesh::{mesh_chunk, AO_MASK, FACE_SHADE_SHIFT},
    world::{Chunk, World, SECTION_SIZE},
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
