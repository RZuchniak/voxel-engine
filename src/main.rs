use raylib::prelude::*;

fn main() {
    let (mut rl, thread) = raylib::init().size(640, 480).title("Hello World").build();

    while !rl.window_should_close() {
        let mut d = rl.begin_drawing(&thread);

        d.clear_background(Color::WHITE);

        {
            let camera = Camera3D::perspective(
                Vector3::new(20.0, 20.0, 20.0),
                Vector3::zero(),
                Vector3::up(),
                45.0,
            );

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
