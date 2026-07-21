//! Greedy-mesher behaviour locked down as exact counts.
//!
//! These guard the optimizations planned for the mesher — removing the per-mesh 5-chunk
//! deep clone, and any vertex-format change. A refactor that is supposed to be
//! behaviour-preserving must not move these numbers.

use voxel_engine::{
    block::BlockId,
    mesh::mesh_chunk_surface,
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
    mesh_chunk_surface(world, coord)
        .iter()
        .map(|(_, mesh)| mesh.indices().len() / 6)
        .sum()
}

#[test]
fn isolated_solid_section_merges_to_six_quads() {
    let mut world = World::new();
    world.insert_chunk(solid_chunk((0, 0), BlockId::STONE));

    let meshes = mesh_chunk_surface(&world, (0, 0));
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
    assert!(mesh_chunk_surface(&world, (0, 0)).is_empty());
}

#[test]
fn missing_chunk_produces_no_mesh() {
    let world = World::new();
    assert!(mesh_chunk_surface(&world, (42, -7)).is_empty());
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
