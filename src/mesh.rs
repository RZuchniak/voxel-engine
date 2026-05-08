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
                if neighbor.is_full_cube() && neighbor.is_opaque() {
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
        let wx = base_x as i32 + primary as i32;
        let wy = base_y as i32 + secondary as i32;
        let wz = base_z as i32 + tertiary as i32;
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
        shade
    };
    let v = |position: [f32; 3], uv: [f32; 2]| Vertex {
        position,
        uv,
        tex_layer,
        light,
    };

    let corners: [Vertex; 4] = match dir {
        Direction::XPositive => {
            let px = (primary + 1) as f32;
            [
                v(pos(px, s0, t0), [0.0, 0.0]),
                v(pos(px, s1, t0), [width as f32, 0.0]),
                v(pos(px, s1, t1), [width as f32, height as f32]),
                v(pos(px, s0, t1), [0.0, height as f32]),
            ]
        }
        Direction::XNegative => {
            let px = primary as f32;
            [
                v(pos(px, s0, t0), [0.0, 0.0]),
                v(pos(px, s0, t1), [0.0, height as f32]),
                v(pos(px, s1, t1), [width as f32, height as f32]),
                v(pos(px, s1, t0), [width as f32, 0.0]),
            ]
        }
        Direction::YPositive => {
            let py = (primary + 1) as f32;
            [
                v(pos(py, s0, t0), [0.0, 0.0]),
                v(pos(py, s1, t0), [width as f32, 0.0]),
                v(pos(py, s1, t1), [width as f32, height as f32]),
                v(pos(py, s0, t1), [0.0, height as f32]),
            ]
        }
        Direction::YNegative => {
            let py = primary as f32;
            [
                v(pos(py, s0, t0), [0.0, 0.0]),
                v(pos(py, s0, t1), [0.0, height as f32]),
                v(pos(py, s1, t1), [width as f32, height as f32]),
                v(pos(py, s1, t0), [width as f32, 0.0]),
            ]
        }
        Direction::ZPositive => {
            let pz = (primary + 1) as f32;
            [
                v(pos(pz, s0, t0), [0.0, 0.0]),
                v(pos(pz, s1, t0), [width as f32, 0.0]),
                v(pos(pz, s1, t1), [width as f32, height as f32]),
                v(pos(pz, s0, t1), [0.0, height as f32]),
            ]
        }
        Direction::ZNegative => {
            let pz = primary as f32;
            [
                v(pos(pz, s0, t0), [0.0, 0.0]),
                v(pos(pz, s0, t1), [0.0, height as f32]),
                v(pos(pz, s1, t1), [width as f32, height as f32]),
                v(pos(pz, s1, t0), [width as f32, 0.0]),
            ]
        }
    };

    mesh.push_quad(corners);
}
