use raylib::consts::*;
use raylib::ffi::{LoadMaterialDefault, Mesh, UploadMesh};
use raylib::prelude::*;
use std::ptr;

pub const CHUNK_SIZE: usize = 16;

#[derive(Default)]
pub struct RustMesh {
    vertex_count: u16,
    vertices: Vec<f32>,
    texcoords: Vec<f32>,
    normals: Vec<f32>,
    tangents: Vec<f32>,
    colors: Vec<u8>,
    indices: Vec<u16>,
}

impl RustMesh {
    fn to_raw_mesh(&mut self) -> Mesh {
        Mesh {
            vertexCount: self.vertex_count as i32,
            triangleCount: (self.indices.len() / 3) as i32,
            vertices: self.vertices.as_mut_ptr(),
            texcoords: self.texcoords.as_mut_ptr(),
            texcoords2: ptr::null_mut(), // Not used in this example
            normals: self.normals.as_mut_ptr(),
            tangents: self.tangents.as_mut_ptr(),
            colors: self.colors.as_mut_ptr(),
            indices: self.indices.as_mut_ptr(),
            animVertices: ptr::null_mut(), // Not used in this example
            animNormals: ptr::null_mut(),  // Not used in this example
            boneIds: ptr::null_mut(),      // Not used in this example
            boneWeights: ptr::null_mut(),  // Not used in this example
            vaoId: 0,                      // Will be set by Raylib
            vboId: ptr::null_mut(),        // Will be set by Raylib
        }
    }

    fn upload_to_gpu(&mut self) -> Mesh {
        let mut raw_mesh = self.to_raw_mesh();
        unsafe {
            UploadMesh(&mut raw_mesh, false);
        }
        raw_mesh
    }
}

#[derive(Clone, Copy)]
struct Position {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl Position {
    pub fn to_vector3(&self) -> Vector3 {
        Vector3::new(self.x as f32, self.y as f32, self.z as f32)
    }
}

#[derive(Clone)]
enum BlockType {
    Default,
    Grass,
    Dirt,
}

#[derive(Clone, Copy)]
pub struct BlockVertex {
    position: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
}

#[derive(Clone, Copy)]
pub struct BlockFace {
    vertices: [BlockVertex; 4],
    indices: [u16; 6],
}

#[derive(Clone)]
struct Block {
    pub m_active: bool,
    pub block_type: BlockType,
}

impl Block {
    pub fn render_block(
        &self,
        chunk_location: Position,
        x: usize,
        y: usize,
        z: usize,
    ) -> Vec<BlockFace> {
        let mut faces = Vec::new();
        let world_x = x as i32 + (CHUNK_SIZE as i32 * chunk_location.x);
        let world_y = y as i32 + (CHUNK_SIZE as i32 * chunk_location.y);
        let world_z = z as i32 + (CHUNK_SIZE as i32 * chunk_location.z);

        faces.push(BlockFace {
            vertices: [
                BlockVertex {
                    position: [
                        world_x as f32 - 1.0,
                        world_y as f32 - 1.0,
                        world_z as f32 + 1.0,
                    ],
                    normal: [0.0, 0.0, 1.0],
                    uv: [0.0, 0.0],
                },
                BlockVertex {
                    position: [
                        world_x as f32 - 1.0,
                        world_y as f32 + 1.0,
                        world_z as f32 + 1.0,
                    ],
                    normal: [0.0, 0.0, 1.0],
                    uv: [0.0, 0.0],
                },
                BlockVertex {
                    position: [
                        world_x as f32 + 1.0,
                        world_y as f32 - 1.0,
                        world_z as f32 + 1.0,
                    ],
                    normal: [0.0, 0.0, 1.0],
                    uv: [0.0, 0.0],
                },
                BlockVertex {
                    position: [
                        world_x as f32 + 1.0,
                        world_y as f32 + 1.0,
                        world_z as f32 + 1.0,
                    ],
                    normal: [0.0, 0.0, 1.0],
                    uv: [0.0, 0.0],
                },
            ],
            indices: [0, 1, 2, 0, 2, 3],
        });

        faces
    }

    pub fn coords_to_location(x: usize, y: usize, z: usize) -> usize {
        x + y * CHUNK_SIZE + z * CHUNK_SIZE * CHUNK_SIZE
    }
}

struct Chunk {
    pub location: Position,
    pub blocks: Vec<Option<Block>>,
    pub needs_update: bool,
}

impl Chunk {
    pub fn new(position: Position) -> Self {
        Self {
            location: position,
            blocks: vec![
                Some(Block {
                    m_active: true,
                    block_type: BlockType::Default
                });
                CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE
            ],
            needs_update: false,
        }
    }

    pub fn create_mesh(&self) -> RustMesh {
        let mut vertex_count = 0;
        let mut rust_mesh = RustMesh::default();

        
        // create some big vector here to store all vertices?
        for x in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                for z in 0..CHUNK_SIZE {
                    if let Some(block) = &self.blocks[Block::coords_to_location(x, y, z)] {
                        if block.m_active {
                            //add vertices from each block
                            let block_faces = block.render_block(self.location, x, y, z);

                            for face in block_faces {
                                for vertex in face.vertices.iter() {
                                    rust_mesh.vertices.extend_from_slice(&vertex.position);
                                    rust_mesh.normals.extend_from_slice(&vertex.normal);
                                    rust_mesh.normals.extend_from_slice(&vertex.uv);
                                }

                                rust_mesh
                                    .indices
                                    .extend(face.indices.iter().map(|i| *i + vertex_count));
                                vertex_count += 4;
                            }
                        }
                    }
                }
            }
        }
        rust_mesh.vertex_count = vertex_count;
        rust_mesh
    }
}

fn main() {
    let (mut rl, thread) = raylib::init().size(1280, 960).title("Voxel Engine").build();

    let mut camera = Camera3D::perspective(
        Vector3::new(20.0, 20.0, 20.0),
        Vector3::zero(),
        Vector3::up(),
        45.0,
    );

    let chunk: Chunk = Chunk::new(Position { x: 0, y: 0, z: 0 });
    let mut rust_mesh = chunk.create_mesh();
    let mesh = rust_mesh.upload_to_gpu();
    let material = unsafe { LoadMaterialDefault() };

    rl.disable_cursor();

    while !rl.window_should_close() {
        if rl.is_key_down(KeyboardKey::KEY_SPACE) {
            camera.position.y += 15.0 * rl.get_frame_time();
            camera.target.y += 15.0 * rl.get_frame_time();
        }
        if rl.is_key_down(KeyboardKey::KEY_LEFT_SHIFT) {
            camera.position.y -= 15.0 * rl.get_frame_time();
            camera.target.y -= 15.0 * rl.get_frame_time();
        }

        rl.update_camera(&mut camera, CameraMode::CAMERA_FIRST_PERSON);

        let mut d = rl.begin_drawing(&thread);

        d.clear_background(Color::WHITE);

        {
            let mut mode3d = d.begin_mode3D(camera);

            mode3d.draw_grid(100, 5.0);

            unsafe {
                raylib::ffi::DrawMesh(mesh, material, Matrix::identity().into());
            }
        }
        d.draw_fps(10, 10);
    }
}
