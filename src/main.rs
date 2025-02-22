use raylib::consts::*;
use raylib::prelude::*;

pub const CHUNK_SIZE: usize = 16;

#[derive(Clone)]
pub struct Position {
    pub x: i32,
    pub y: i32,
    pub z: i32
}

impl Position {
    pub fn new(x: i32, y: i32, z: i32) {
        Self {x, y, z};
    }

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



#[derive(Clone)]

struct Block {
    pub location: Position,
    pub m_active: bool,
    pub block_type: BlockType,
}

impl Block {
    pub fn create_block(&self, rl: &RaylibHandle, thread: &RaylibThread, chunk_location: Position) -> Vec<Vec<Position>> {
        let world_x = self.location.x + (CHUNK_SIZE as i32* chunk_location.x);
        let world_y = self.location.y + (CHUNK_SIZE as i32* chunk_location.y);
        let world_z = self.location.z + (CHUNK_SIZE as i32* chunk_location.z);

        let p1 = [world_x - 1, world_y - 1, world_z + 1];
        let p2 = [world_x + 1, world_y - 1, world_z + 1];
        let p3 = [world_x + 1, world_y + 1, world_z + 1];
        let p4 = [world_x - 1, world_y + 1, world_z + 1];
        let p5 = [world_x + 1, world_y - 1, world_z - 1];
        let p6 = [world_x - 1, world_y - 1, world_z - 1];
        let p7 = [world_x - 1, world_y + 1, world_z - 1];
        let p8 = [world_x + 1, world_y + 1, world_z - 1];
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
            blocks: vec![None; CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE],
            needs_update: false,
        }
    }

    pub fn create_mesh(&self, rl: &RaylibHandle, thread: &RaylibThread) -> Model {
        for x in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                for z in 0..CHUNK_SIZE {
                    if let Some(block) = &self.blocks[Block::coords_to_location(x, y, z)] {
                        if block.m_active {
                            block.create_block(rl, thread, self.location)
                        }
                    }
                }
            }
        }
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

            mode3d.draw_grid(10, 1.0);

            for x in 1..10 {
                for y in 1..10 {
                    for z in 1..10 {
                        mode3d.draw_cube(
                            Vector3::new(x as f32, y as f32, z as f32),
                            0.5,
                            0.5,
                            0.5,
                            Color::RED,
                        );
                    }
                }
            }
        }
    }
}
