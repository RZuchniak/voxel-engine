use raylib::consts::*;
use raylib::prelude::*;

enum BlockType {
    Default,
    Grass,
    Dirt,
}

struct Block {
    m_active: bool,
    block_type: BlockType,
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
            camera.position.y += 1.0*rl.get_frame_time();
            camera.target.y += 1.0*rl.get_frame_time();
        }
        if rl.is_key_down(KeyboardKey::KEY_LEFT_SHIFT) {
            camera.position.y -= 1.0*rl.get_frame_time();
            camera.target.y -= 1.0*rl.get_frame_time();
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
