use crate::{BlockType, Vertex};

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

pub struct Chunk {
    data: [BlockType; 16 * 16 * 16],
    position: (i32, i32, i32),
    mesh: Option<MeshData>,
}

impl Chunk {
    pub fn new(position: (i32, i32, i32)) -> Self {
        Self {
            data: [BlockType::AIR; 16 * 16 * 16],
            position,
            mesh: None,
        }
    }

    fn index(x: usize, y: usize, z: usize) -> usize {
        x + y * 16 + z * 16 * 16
    }

    fn set_block(&mut self, x: usize, y: usize, z: usize, block: BlockType) {
        if x < 16 && y < 16 && z < 16 {
            let idx = Self::index(x, y, z);
            self.data[idx] = block;
        }
    }

    pub fn generate_test_chunk(&mut self) {
        // Simple solid floor of stone at y = 0
        for z in 0..16 {
            for x in 0..16 {
                self.set_block(x, 0, z, BlockType::STONE);
            }
        }

        // Small grass hill to see vertical faces
        for y in 1..4 {
            for z in 4..12 {
                for x in 4..12 {
                    if y == 3 {
                        self.set_block(x, y, z, BlockType::GRASS);
                    } else {
                        self.set_block(x, y, z, BlockType::STONE);
                    }
                }
            }
        }
    }

    pub fn mesh(&self) -> Option<&MeshData> {
        self.mesh.as_ref()
    }

    pub fn generate_mesh(&mut self) {
        let mut mesh_data = MeshData::new();
        self.mesh_direction(&mut mesh_data, Direction::XPositive);
        self.mesh_direction(&mut mesh_data, Direction::XNegative);
        self.mesh_direction(&mut mesh_data, Direction::YPositive);
        self.mesh_direction(&mut mesh_data, Direction::YNegative);
        self.mesh_direction(&mut mesh_data, Direction::ZPositive);
        self.mesh_direction(&mut mesh_data, Direction::ZNegative);
        self.mesh = Some(mesh_data);
    }

    #[inline]
    fn get_block(&self, x: usize, y: usize, z: usize) -> BlockType {
        if x >= 16 || y >= 16 || z >= 16 {
            BlockType::AIR
        } else {
            self.data[x + y * 16 + z * 16 * 16]
        }
    }

    #[inline]
    fn block_at_i32(&self, x: i32, y: i32, z: i32) -> BlockType {
        if x < 0 || y < 0 || z < 0 || x >= 16 || y >= 16 || z >= 16 {
            BlockType::AIR
        } else {
            self.get_block(x as usize, y as usize, z as usize)
        }
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

    fn get_block_color(block_type: BlockType) -> [f32; 3] {
        match block_type {
            BlockType::AIR => [0.0, 0.0, 0.0],
            BlockType::STONE => [0.5, 0.5, 0.5],
            BlockType::GRASS => [0.0, 1.0, 0.0],
        }
    }

    fn mesh_direction(&self, mesh: &mut MeshData, dir: Direction) {
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

        for primary in 0usize..16 {
            let mut merged = [[false; 16]; 16];

            for secondary in 0usize..16 {
                for tertiary in 0usize..16 {
                    if merged[secondary][tertiary] {
                        continue;
                    }

                    let (cx, cy, cz) = Self::unpack_coords(
                        primary_axis,
                        secondary_axis,
                        tertiary_axis,
                        primary,
                        secondary,
                        tertiary,
                    );
                    let cx = cx as i32;
                    let cy = cy as i32;
                    let cz = cz as i32;

                    let block = self.block_at_i32(cx, cy, cz);
                    if matches!(block, BlockType::AIR) {
                        continue;
                    }

                    if !matches!(self.block_at_i32(cx + ox, cy + oy, cz + oz), BlockType::AIR) {
                        continue;
                    }

                    let color = Self::get_block_color(block);

                    let mut width = 1usize;
                    while secondary + width < 16 && !merged[secondary + width][tertiary] {
                        let (nx, ny, nz) = Self::unpack_coords(
                            primary_axis,
                            secondary_axis,
                            tertiary_axis,
                            primary,
                            secondary + width,
                            tertiary,
                        );
                        let nx = nx as i32;
                        let ny = ny as i32;
                        let nz = nz as i32;
                        if self.block_at_i32(nx, ny, nz) != block {
                            break;
                        }
                        if !matches!(
                            self.block_at_i32(nx + ox, ny + oy, nz + oz),
                            BlockType::AIR
                        ) {
                            break;
                        }
                        width += 1;
                    }

                    let mut height = 1usize;
                    'grow: while tertiary + height < 16 {
                        for w in 0..width {
                            if merged[secondary + w][tertiary + height] {
                                break 'grow;
                            }
                            let (nx, ny, nz) = Self::unpack_coords(
                                primary_axis,
                                secondary_axis,
                                tertiary_axis,
                                primary,
                                secondary + w,
                                tertiary + height,
                            );
                            let nx = nx as i32;
                            let ny = ny as i32;
                            let nz = nz as i32;
                            if self.block_at_i32(nx, ny, nz) != block {
                                break 'grow;
                            }
                            if !matches!(
                                self.block_at_i32(nx + ox, ny + oy, nz + oz),
                                BlockType::AIR
                            ) {
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

                    self.emit_quad(
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
                        color,
                    );
                }
            }
        }
    }

    fn emit_quad(
        &self,
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
        color: [f32; 3],
    ) {
        let (bx, by, bz) = self.position;
        let base_x = bx as f32 * 16.0;
        let base_y = by as f32 * 16.0;
        let base_z = bz as f32 * 16.0;

        let s0 = secondary as f32;
        let s1 = (secondary + width) as f32;
        let t0 = tertiary as f32;
        let t1 = (tertiary + height) as f32;

        let pos = |pa: f32, sb: f32, tb: f32| -> [f32; 3] {
            let mut p = [0.0f32; 3];
            p[primary_axis] = pa;
            p[secondary_axis] = sb;
            p[tertiary_axis] = tb;
            [
                base_x + p[0],
                base_y + p[1],
                base_z + p[2],
            ]
        };

        let corners: [Vertex; 4] = match dir {
            Direction::XPositive => {
                let px = (primary + 1) as f32;
                [
                    Vertex {
                        position: pos(px, s0, t0),
                        color,
                    },
                    Vertex {
                        position: pos(px, s1, t0),
                        color,
                    },
                    Vertex {
                        position: pos(px, s1, t1),
                        color,
                    },
                    Vertex {
                        position: pos(px, s0, t1),
                        color,
                    },
                ]
            }
            Direction::XNegative => {
                let px = primary as f32;
                [
                    Vertex {
                        position: pos(px, s0, t0),
                        color,
                    },
                    Vertex {
                        position: pos(px, s0, t1),
                        color,
                    },
                    Vertex {
                        position: pos(px, s1, t1),
                        color,
                    },
                    Vertex {
                        position: pos(px, s1, t0),
                        color,
                    },
                ]
            }
            Direction::YPositive => {
                let py = (primary + 1) as f32;
                [
                    Vertex {
                        position: pos(py, s0, t0),
                        color,
                    },
                    Vertex {
                        position: pos(py, s1, t0),
                        color,
                    },
                    Vertex {
                        position: pos(py, s1, t1),
                        color,
                    },
                    Vertex {
                        position: pos(py, s0, t1),
                        color,
                    },
                ]
            }
            Direction::YNegative => {
                let py = primary as f32;
                [
                    Vertex {
                        position: pos(py, s0, t0),
                        color,
                    },
                    Vertex {
                        position: pos(py, s0, t1),
                        color,
                    },
                    Vertex {
                        position: pos(py, s1, t1),
                        color,
                    },
                    Vertex {
                        position: pos(py, s1, t0),
                        color,
                    },
                ]
            }
            Direction::ZPositive => {
                let pz = (primary + 1) as f32;
                [
                    Vertex {
                        position: pos(pz, s0, t0),
                        color,
                    },
                    Vertex {
                        position: pos(pz, s1, t0),
                        color,
                    },
                    Vertex {
                        position: pos(pz, s1, t1),
                        color,
                    },
                    Vertex {
                        position: pos(pz, s0, t1),
                        color,
                    },
                ]
            }
            Direction::ZNegative => {
                let pz = primary as f32;
                [
                    Vertex {
                        position: pos(pz, s0, t0),
                        color,
                    },
                    Vertex {
                        position: pos(pz, s0, t1),
                        color,
                    },
                    Vertex {
                        position: pos(pz, s1, t1),
                        color,
                    },
                    Vertex {
                        position: pos(pz, s1, t0),
                        color,
                    },
                ]
            }
        };

        mesh.push_quad(corners);
    }
}
