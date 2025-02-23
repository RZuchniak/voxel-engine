use raylib::consts::*;
use raylib::ffi::{LoadMaterialDefault, Mesh, UploadMesh};
use raylib::prelude::*;
use std::ptr;

pub const CHUNK_SIZE: usize = 16;
pub const BLOCK_SIZE: usize = 1;

#[derive(Default, Debug)]
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
        rust_mesh: &mut RustMesh,
    ) {
        let world_x = x as f32 + (CHUNK_SIZE as f32 * chunk_location.x as f32);
        let world_y = y as f32 + (CHUNK_SIZE as f32 * chunk_location.y as f32);
        let world_z = z as f32 + (CHUNK_SIZE as f32 * chunk_location.z as f32);

        rust_mesh.vertices.extend_from_slice(&[
            // Front face
            world_x - 0.5,
            world_y - 0.5,
            world_z + 0.5, // 0
            world_x + 0.5,
            world_y - 0.5,
            world_z + 0.5, // 1
            world_x + 0.5,
            world_y + 0.5,
            world_z + 0.5, // 2
            world_x - 0.5,
            world_y + 0.5,
            world_z + 0.5, // 3
            // Back face
            world_x - 0.5,
            world_y - 0.5,
            world_z - 0.5, // 4
            world_x + 0.5,
            world_y - 0.5,
            world_z - 0.5, // 5
            world_x + 0.5,
            world_y + 0.5,
            world_z - 0.5, // 6
            world_x - 0.5,
            world_y + 0.5,
            world_z - 0.5, // 7
        ]);

        // Texture coordinates (mapped for each vertex)
        rust_mesh.texcoords.extend_from_slice(&[
            0.0, 1.0, // 0
            1.0, 1.0, // 1
            1.0, 0.0, // 2
            0.0, 0.0, // 3
            0.0, 1.0, // 4
            1.0, 1.0, // 5
            1.0, 0.0, // 6
            0.0, 0.0, // 7
        ]);

        // Normals (one normal per vertex, pointing outwards)
        rust_mesh.normals.extend_from_slice(&[
            0.0, 0.0, 1.0, // Front face normals
            0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, -1.0, // Back face normals
            0.0, 0.0, -1.0, 0.0, 0.0, -1.0, 0.0, 0.0, -1.0,
        ]);

        // Tangents (set to zero or calculate for advanced lighting)
        rust_mesh.tangents.extend_from_slice(&[0.0; 8 * 4]); // 4 floats per tangent, 8 vertices

        // Colors (RGBA - white)
        rust_mesh.colors.extend_from_slice(&[
            255, 255, 255, 255, // 0
            255, 255, 255, 255, // 1
            255, 255, 255, 255, // 2
            255, 255, 255, 255, // 3
            255, 255, 255, 255, // 4
            255, 255, 255, 255, // 5
            255, 255, 255, 255, // 6
            255, 255, 255, 255, // 7
        ]);

        // Indices (12 triangles to form the cube)
        let indices = [
            // Front face
            0, 1, 2, 2, 3, 0, // Right face
            1, 5, 6, 6, 2, 1, // Back face
            5, 4, 7, 7, 6, 5, // Left face
            4, 0, 3, 3, 7, 4, // Top face
            3, 2, 6, 6, 7, 3, // Bottom face
            4, 5, 1, 1, 0, 4,
        ];
        rust_mesh
            .indices
            .extend(indices.iter().map(|i| *i + rust_mesh.vertex_count));

        rust_mesh.vertex_count += 8;
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

        let expected_faces = CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE * 6; // 6 faces per block max
        rust_mesh.vertices = Vec::with_capacity(expected_faces * 24); // 4 vertices * 3 coordinates
        rust_mesh.normals = Vec::with_capacity(expected_faces * 24);
        rust_mesh.texcoords = Vec::with_capacity(expected_faces * 16); // 4 vertices * 2 coordinates
        rust_mesh.indices = Vec::with_capacity(expected_faces * 12); // 6 indices per face
        rust_mesh.colors = vec![255; expected_faces * 32]; // 4 vertices * 4 colors (RGBA)

        // create some big vector here to store all vertices?
        for x in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                for z in 0..CHUNK_SIZE {
                    if let Some(block) = &self.blocks[Block::coords_to_location(x, y, z)] {
                        if block.m_active {
                            //add vertices from each block
                            block.render_block(self.location, x, y, z, &mut rust_mesh);
                        }
                    }
                }
            }
        }
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
    let mut rust_mesh1 = chunk.create_mesh();
    let mesh = rust_mesh1.upload_to_gpu();

    let vertices = vec![
        // Front face
        -0.5, -0.5, 0.5, // 0
        0.5, -0.5, 0.5, // 1
        0.5, 0.5, 0.5, // 2
        -0.5, 0.5, 0.5, // 3
        // Back face
        -0.5, -0.5, -0.5, // 4
        0.5, -0.5, -0.5, // 5
        0.5, 0.5, -0.5, // 6
        -0.5, 0.5, -0.5, // 7
    ];

    // Texture coordinates (mapped for each vertex)
    let texcoords = vec![
        0.0, 1.0, // 0
        1.0, 1.0, // 1
        1.0, 0.0, // 2
        0.0, 0.0, // 3
        0.0, 1.0, // 4
        1.0, 1.0, // 5
        1.0, 0.0, // 6
        0.0, 0.0, // 7
    ];

    // Normals (one normal per vertex, pointing outwards)
    let normals = vec![
        0.0, 0.0, 1.0, // Front face normals
        0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, -1.0, // Back face normals
        0.0, 0.0, -1.0, 0.0, 0.0, -1.0, 0.0, 0.0, -1.0,
    ];

    // Tangents (set to zero or calculate for advanced lighting)
    let tangents = vec![0.0; 8 * 4]; // 4 floats per tangent, 8 vertices

    // Colors (RGBA - white)
    let colors = vec![
        255, 255, 255, 255, // 0
        255, 255, 255, 255, // 1
        255, 255, 255, 255, // 2
        255, 255, 255, 255, // 3
        255, 255, 255, 255, // 4
        255, 255, 255, 255, // 5
        255, 255, 255, 255, // 6
        255, 255, 255, 255, // 7
    ];

    // Indices (12 triangles to form the cube)
    let indices = vec![
        // Front face
        0, 1, 2, 2, 3, 0, // Right face
        1, 5, 6, 6, 2, 1, // Back face
        5, 4, 7, 7, 6, 5, // Left face
        4, 0, 3, 3, 7, 4, // Top face
        3, 2, 6, 6, 7, 3, // Bottom face
        4, 5, 1, 1, 0, 4,
    ];

    let mut rust_mesh2 = RustMesh {
        vertex_count: 8,
        vertices,
        texcoords,
        normals,
        tangents,
        colors,
        indices,
    };

    // let mesh = rust_mesh2.upload_to_gpu();

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

        d.clear_background(Color::BLACK);

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
