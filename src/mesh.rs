use crate::{
    block::Face,
    Vertex,
    world::{MIN_SECTION_Y, SECTION_SIZE, World},
};

pub struct MeshData {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
}

impl MeshData {
    pub fn new() -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty()
    }

    pub fn vertices(&self) -> &[Vertex] {
        &self.vertices
    }

    pub fn indices(&self) -> &[u32] {
        &self.indices
    }

    pub fn from_wire_bytes(vertices: Vec<u8>, indices: Vec<u8>) -> Option<Self> {
        let vertex_size = std::mem::size_of::<Vertex>();
        if vertices.len() % vertex_size != 0 {
            return None;
        }
        if indices.len() % std::mem::size_of::<u32>() != 0 {
            return None;
        }
        let vertices = bytemuck::try_cast_vec(vertices).ok()?;
        let indices = bytemuck::try_cast_vec(indices).ok()?;
        Some(Self { vertices, indices })
    }

    pub fn to_wire_bytes(&self) -> (Vec<u8>, Vec<u8>) {
        (
            bytemuck::cast_slice(&self.vertices).to_vec(),
            bytemuck::cast_slice(&self.indices).to_vec(),
        )
    }

    fn push_quad(&mut self, corners: [Vertex; 4]) {
        let base = self.vertices.len() as u32;
        self.vertices.extend_from_slice(&corners);
        self.indices.extend_from_slice(&[
            base,
            base + 1,
            base + 2,
            base,
            base + 2,
            base + 3,
        ]);
    }
}

#[derive(Clone, Copy)]
enum Direction {
    XPositive,
    XNegative,
    YPositive,
    YNegative,
    ZPositive,
    ZNegative,
}

impl Direction {
    /// Which of vanilla's four fixed face brightnesses this face takes.
    ///
    /// Minecraft multiplies every face by a constant that depends only on its direction — up 1.0,
    /// down 0.5, north/south 0.8, east/west 0.6 (`LightUtil`/`FaceInfo` shading). It is what makes
    /// a cube read as a cube under a single sky light: without it every face of a block is the
    /// same colour and the geometry goes flat. The engine had no directional term at all; its only
    /// shading input was the per-vertex occluder count below, which varies with *position* rather
    /// than facing, so it produced blotches instead of form.
    ///
    /// Returned as an index rather than a factor because it is packed into the light word and
    /// resolved in the shader — see [`FACE_SHADE_SHIFT`].
    #[inline]
    fn shade_index(self) -> u32 {
        match self {
            Direction::YPositive => 0,
            Direction::YNegative => 1,
            Direction::ZPositive | Direction::ZNegative => 2,
            Direction::XPositive | Direction::XNegative => 3,
        }
    }
}

/// The vertex `light` word is `ao | face_shade_index << FACE_SHADE_SHIFT`: the low byte is the
/// interpolated per-vertex occlusion, the next two bits pick the flat per-face brightness. Packed
/// into the existing `u32` attribute so this costs no extra vertex bandwidth — `square.wgsl`
/// unpacks both halves and must agree with these constants.
pub const FACE_SHADE_SHIFT: u32 = 8;
pub const AO_MASK: u32 = 0xFF;

#[inline]
fn unpack_coords(
    primary_axis: usize,
    secondary_axis: usize,
    tertiary_axis: usize,
    p: usize,
    s: usize,
    t: usize,
) -> (usize, usize, usize) {
    let mut c = [0usize; 3];
    c[primary_axis] = p;
    c[secondary_axis] = s;
    c[tertiary_axis] = t;
    (c[0], c[1], c[2])
}

pub fn mesh_section(world: &World, chunk_coord: (i32, i32), section_index: usize) -> Option<MeshData> {
    let mut mesh_data = MeshData::new();
    mesh_direction(
        world,
        &mut mesh_data,
        chunk_coord,
        section_index,
        Direction::XPositive,
    );
    mesh_direction(
        world,
        &mut mesh_data,
        chunk_coord,
        section_index,
        Direction::XNegative,
    );
    mesh_direction(
        world,
        &mut mesh_data,
        chunk_coord,
        section_index,
        Direction::YPositive,
    );
    mesh_direction(
        world,
        &mut mesh_data,
        chunk_coord,
        section_index,
        Direction::YNegative,
    );
    mesh_direction(
        world,
        &mut mesh_data,
        chunk_coord,
        section_index,
        Direction::ZPositive,
    );
    mesh_direction(
        world,
        &mut mesh_data,
        chunk_coord,
        section_index,
        Direction::ZNegative,
    );

    if mesh_data.is_empty() {
        None
    } else {
        Some(mesh_data)
    }
}

/// Mesh every populated section of a chunk.
///
/// The mesher deliberately has **no depth heuristic of its own**: it draws exactly what the
/// source handed it. Deciding here how much of a chunk is worth meshing means guessing at
/// what the camera can see from block data alone, and every such guess has been wrong —
/// anchored at the chunk's highest block it dropped the ground under every slope. Occlusion
/// is [`crate::cull::SectionGraph`]'s job; trimming, where a source wants it, belongs to the
/// source.
pub fn mesh_chunk(world: &World, chunk_coord: (i32, i32)) -> Vec<(usize, MeshData)> {
    let Some(chunk) = world.chunk(chunk_coord) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for section_index in chunk.populated_section_indices() {
        if let Some(mesh) = mesh_section(world, chunk_coord, section_index) {
            out.push((section_index, mesh));
        }
    }
    out
}

fn mesh_direction(
    world: &World,
    mesh: &mut MeshData,
    chunk_coord: (i32, i32),
    section_index: usize,
    dir: Direction,
) {
        let (
            primary_axis,
            secondary_axis,
            tertiary_axis,
            direction_offset,
            _is_positive,
        ) = match dir {
            Direction::XPositive => (0usize, 1, 2, (1, 0, 0), true),
            Direction::XNegative => (0, 1, 2, (-1, 0, 0), false),
            Direction::YPositive => (1, 2, 0, (0, 1, 0), true),
            Direction::YNegative => (1, 2, 0, (0, -1, 0), false),
            Direction::ZPositive => (2, 0, 1, (0, 0, 1), true),
            Direction::ZNegative => (2, 0, 1, (0, 0, -1), false),
        };

    let (ox, oy, oz) = direction_offset;
    let world_chunk_x = chunk_coord.0 * SECTION_SIZE as i32;
    let world_chunk_z = chunk_coord.1 * SECTION_SIZE as i32;
    let section_world_y = (section_index as i32 + MIN_SECTION_Y) * SECTION_SIZE as i32;

    for primary in 0usize..SECTION_SIZE {
        let mut merged = [[false; SECTION_SIZE]; SECTION_SIZE];

        for secondary in 0usize..SECTION_SIZE {
            for tertiary in 0usize..SECTION_SIZE {
                if merged[secondary][tertiary] {
                    continue;
                }

                let (sx, sy, sz) = unpack_coords(
                    primary_axis,
                    secondary_axis,
                    tertiary_axis,
                    primary,
                    secondary,
                    tertiary,
                );
                let wx = world_chunk_x + sx as i32;
                let wy = section_world_y + sy as i32;
                let wz = world_chunk_z + sz as i32;

                let block = world.block_at(wx, wy, wz);
                if block.is_air() || !block.is_full_cube() {
                    continue;
                }

                let neighbor = world.block_at(wx + ox, wy + oy, wz + oz);
                if neighbor == block {
                    continue;
                }
                if neighbor.is_full_cube() && neighbor.occludes_faces() {
                    continue;
                }
                // Two *different* non-occluding full cubes share a plane, and both are emitted
                // double-sided, so without a tie-break each draws a face there and the pair
                // z-fights — a shimmer that resolves in the nearer block's favour as you approach.
                // Culling the higher id is deterministic and view-independent, and leaves exactly
                // one face, which being double-sided still reads correctly from both sides.
                //
                // Only reachable when the neighbour does not occlude (the test above returned) and
                // this block does not either — so it never touches an opaque block's face.
                if neighbor.is_full_cube() && !block.occludes_faces() && block.0 > neighbor.0 {
                    continue;
                }

                let face = match dir {
                    Direction::YPositive => Face::Top,
                    Direction::YNegative => Face::Bottom,
                    _ => Face::Side,
                };
                let tex_layer = block.texture_layer(face);

                let mut width = 1usize;
                while secondary + width < SECTION_SIZE && !merged[secondary + width][tertiary] {
                    let (nx, ny, nz) = unpack_coords(
                        primary_axis,
                        secondary_axis,
                        tertiary_axis,
                        primary,
                        secondary + width,
                        tertiary,
                    );
                    let nwx = world_chunk_x + nx as i32;
                    let nwy = section_world_y + ny as i32;
                    let nwz = world_chunk_z + nz as i32;
                    let nblock = world.block_at(nwx, nwy, nwz);
                    if nblock != block {
                        break;
                    }
                    let nneighbor = world.block_at(nwx + ox, nwy + oy, nwz + oz);
                    if nneighbor == nblock {
                        break;
                    }
                    if nneighbor.is_full_cube() && nneighbor.is_opaque() {
                        break;
                    }
                    width += 1;
                }

                let mut height = 1usize;
                'grow: while tertiary + height < SECTION_SIZE {
                    for w in 0..width {
                        if merged[secondary + w][tertiary + height] {
                            break 'grow;
                        }
                        let (nx, ny, nz) = unpack_coords(
                            primary_axis,
                            secondary_axis,
                            tertiary_axis,
                            primary,
                            secondary + w,
                            tertiary + height,
                        );
                        let nwx = world_chunk_x + nx as i32;
                        let nwy = section_world_y + ny as i32;
                        let nwz = world_chunk_z + nz as i32;
                        let nblock = world.block_at(nwx, nwy, nwz);
                        if nblock != block {
                            break 'grow;
                        }
                        let nneighbor = world.block_at(nwx + ox, nwy + oy, nwz + oz);
                        if nneighbor == nblock {
                            break 'grow;
                        }
                        if nneighbor.is_full_cube() && nneighbor.is_opaque() {
                            break 'grow;
                        }
                    }
                    height += 1;
                }

                for yy in 0..height {
                    for xx in 0..width {
                        merged[secondary + xx][tertiary + yy] = true;
                    }
                }

                emit_quad(
                    world,
                    mesh,
                    dir,
                    primary_axis,
                    secondary_axis,
                    tertiary_axis,
                    primary,
                    secondary,
                    tertiary,
                    width,
                    height,
                    tex_layer,
                    world_chunk_x as f32,
                    section_world_y as f32,
                    world_chunk_z as f32,
                    // Water, ice and lava are full cubes you can stand inside, so their
                    // surfaces have to exist from both sides — otherwise backface culling
                    // makes the water surface vanish when seen from underwater.
                    !block.is_opaque(),
                );
            }
        }
    }
}

fn emit_quad(
    world: &World,
    mesh: &mut MeshData,
    dir: Direction,
    primary_axis: usize,
    secondary_axis: usize,
    tertiary_axis: usize,
    primary: usize,
    secondary: usize,
    tertiary: usize,
    width: usize,
    height: usize,
    tex_layer: u32,
    base_x: f32,
    base_y: f32,
    base_z: f32,
    double_sided: bool,
) {
    let s0 = secondary as f32;
    let s1 = (secondary + width) as f32;
    let t0 = tertiary as f32;
    let t1 = (tertiary + height) as f32;

    let pos = |pa: f32, sb: f32, tb: f32| -> [f32; 3] {
        let mut p = [0.0f32; 3];
        p[primary_axis] = pa;
        p[secondary_axis] = sb;
        p[tertiary_axis] = tb;
        [base_x + p[0], base_y + p[1], base_z + p[2]]
    };

    // Approximate AO/brightness input inspired by stb_voxel_render's per-vertex lighting.
    let light = {
        let (lx, ly, lz) = unpack_coords(
            primary_axis,
            secondary_axis,
            tertiary_axis,
            primary,
            secondary,
            tertiary,
        );
        let wx = base_x as i32 + lx as i32;
        let wy = base_y as i32 + ly as i32;
        let wz = base_z as i32 + lz as i32;
        let mut occluders = 0u32;
        for (dx, dy, dz) in [
            (1, 0, 0),
            (-1, 0, 0),
            (0, 1, 0),
            (0, -1, 0),
            (0, 0, 1),
            (0, 0, -1),
        ] {
            let n = world.block_at(wx + dx, wy + dy, wz + dz);
            if n.is_opaque() && n.is_full_cube() {
                occluders += 1;
            }
        }
        let shade = (255u32.saturating_sub(occluders * 28)).max(72);
        debug_assert!(shade <= AO_MASK, "AO must fit the low byte of the light word");
        shade | (dir.shade_index() << FACE_SHADE_SHIFT)
    };
    let v = |position: [f32; 3], uv: [f32; 2]| Vertex {
        position,
        uv,
        tex_layer,
        light,
    };

    // Minecraft-style UVs from world position (Repeat sampler tiles every block).
    let uv = |p: [f32; 3]| -> [f32; 2] {
        match dir {
            Direction::XPositive => [p[2], -p[1]],
            Direction::XNegative => [-p[2], -p[1]],
            Direction::YPositive => [p[0], p[2]],
            Direction::YNegative => [p[0], -p[2]],
            Direction::ZPositive => [p[0], -p[1]],
            Direction::ZNegative => [-p[0], -p[1]],
        }
    };

    let mut corners: [Vertex; 4] = match dir {
        Direction::XPositive => {
            let px = (primary + 1) as f32;
            [
                v(pos(px, s0, t0), uv(pos(px, s0, t0))),
                v(pos(px, s1, t0), uv(pos(px, s1, t0))),
                v(pos(px, s1, t1), uv(pos(px, s1, t1))),
                v(pos(px, s0, t1), uv(pos(px, s0, t1))),
            ]
        }
        Direction::XNegative => {
            let px = primary as f32;
            [
                v(pos(px, s0, t0), uv(pos(px, s0, t0))),
                v(pos(px, s0, t1), uv(pos(px, s0, t1))),
                v(pos(px, s1, t1), uv(pos(px, s1, t1))),
                v(pos(px, s1, t0), uv(pos(px, s1, t0))),
            ]
        }
        Direction::YPositive => {
            let py = (primary + 1) as f32;
            [
                v(pos(py, s0, t0), uv(pos(py, s0, t0))),
                v(pos(py, s1, t0), uv(pos(py, s1, t0))),
                v(pos(py, s1, t1), uv(pos(py, s1, t1))),
                v(pos(py, s0, t1), uv(pos(py, s0, t1))),
            ]
        }
        Direction::YNegative => {
            let py = primary as f32;
            [
                v(pos(py, s0, t0), uv(pos(py, s0, t0))),
                v(pos(py, s0, t1), uv(pos(py, s0, t1))),
                v(pos(py, s1, t1), uv(pos(py, s1, t1))),
                v(pos(py, s1, t0), uv(pos(py, s1, t0))),
            ]
        }
        Direction::ZPositive => {
            let pz = (primary + 1) as f32;
            [
                v(pos(pz, s0, t0), uv(pos(pz, s0, t0))),
                v(pos(pz, s1, t0), uv(pos(pz, s1, t0))),
                v(pos(pz, s1, t1), uv(pos(pz, s1, t1))),
                v(pos(pz, s0, t1), uv(pos(pz, s0, t1))),
            ]
        }
        Direction::ZNegative => {
            let pz = primary as f32;
            [
                v(pos(pz, s0, t0), uv(pos(pz, s0, t0))),
                v(pos(pz, s0, t1), uv(pos(pz, s0, t1))),
                v(pos(pz, s1, t1), uv(pos(pz, s1, t1))),
                v(pos(pz, s1, t0), uv(pos(pz, s1, t0))),
            ]
        }
    };

    // Corners stay exactly on the block lattice — see `BACK_FACE_INSET` for why nudging a face
    // along its own normal is not a free "hide the seams" trick but the cause of them.
    mesh.push_quad(corners);

    if double_sided {
        // Reversing the corner cycle flips the winding, so this copy survives backface culling
        // exactly when the front one does not — meaning only ever one of the pair rasterizes
        // and the two cannot z-fight. The inset is only insurance for culling being turned off,
        // and it is invisible because it moves the face towards a viewer already inside the
        // fluid.
        let mut back = corners;
        back.reverse();
        shift_corners(&mut back, dir, -BACK_FACE_INSET);
        mesh.push_quad(back);
    }
}

/// How far the reversed copy of a fluid surface sits *inside* the block.
///
/// Every face used to be pushed out along its normal by this much, on the theory that it hid
/// seams. It did the opposite: two perpendicular faces of the same block each moved away from
/// their shared edge, so at every **convex** edge the planes miss each other and leave a slot of
/// this width running the length of the edge. You see straight through it — against the sky
/// clear colour that reads as a bright hairline outlining every block, which is exactly the
/// reported bug. Concave edges were fine, which is why it only showed on silhouettes.
///
/// Faces that meet on the lattice rasterize watertight: adjacent triangles sharing an edge with
/// bit-identical vertices have no gap by the rasterization rules. There was nothing to hide.
const BACK_FACE_INSET: f32 = 0.002;

fn shift_corners(corners: &mut [Vertex; 4], dir: Direction, amount: f32) {
    let (nx, ny, nz) = match dir {
        Direction::XPositive => (amount, 0.0, 0.0),
        Direction::XNegative => (-amount, 0.0, 0.0),
        Direction::YPositive => (0.0, amount, 0.0),
        Direction::YNegative => (0.0, -amount, 0.0),
        Direction::ZPositive => (0.0, 0.0, amount),
        Direction::ZNegative => (0.0, 0.0, -amount),
    };
    for corner in corners.iter_mut() {
        corner.position[0] += nx;
        corner.position[1] += ny;
        corner.position[2] += nz;
    }
}
