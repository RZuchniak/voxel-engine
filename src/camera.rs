use cgmath::{InnerSpace, Matrix4, Vector3};

#[derive(Clone)]
pub struct Camera {
    position: cgmath::Point3<f32>,
    target: cgmath::Point3<f32>,
    up: cgmath::Vector3<f32>,
    pub aspect_ratio: f32,
    fov: f32,
    znear: f32,
    zfar: f32,
    pitch: f64,
    yaw: f64,
}

impl Camera {
    pub fn build_view_projection_matrix(&self) -> cgmath::Matrix4<f32> {
        let view = cgmath::Matrix4::look_at_rh(self.position, self.target, self.up);
        let proj = cgmath::perspective(
            cgmath::Deg(self.fov),
            self.aspect_ratio,
            self.znear,
            self.zfar,
        );
        proj * view // * OPENGL_TO_WGPU_MATRIX
    }

    pub fn new(aspect_ratio: f32, fov: f32, znear: f32, zfar: f32) -> Self {
        Self {
            position: (-10.5, 0.5, 0.5).into(),
            target: (-9.5, 0.5, 0.5).into(),
            up: cgmath::Vector3::unit_y(),
            aspect_ratio,
            fov,
            znear,
            zfar,
            pitch: 0.0 as f64,
            yaw: 0.0 as f64,
        }
    }
}

pub struct Controller {
    pub speed: u128,
    pub sensitivity: f32,

    pub forward: bool,
    pub backward: bool,
    pub left: bool,
    pub right: bool,
    pub up: bool,
    pub down: bool,

    pub mouse_delta: (f64, f64),
}

impl Controller {
    pub fn new(speed: u128, sensitivity: f32) -> Self {
        Self {
            speed,
            sensitivity,
            forward: false,
            backward: false,
            left: false,
            right: false,
            up: false,
            down: false,
            mouse_delta: (0.0, 0.0),
        }
    }

    pub fn update(&mut self, delta_time: u128, camera: &mut Camera) {
        let previous = camera.clone();
        let speed = self.speed * delta_time;
        let sensitivity = self.sensitivity * delta_time as f32;
        let diff = previous.target - previous.position;

        if self.forward {
            camera.position += diff * speed as f32 / 100000.0;
        }
        if self.backward {
            camera.position -= diff * speed as f32 / 100000.0;
        }
        if self.left {
            camera.position -= diff.cross(previous.up) * speed as f32 / 100000.0;
        }
        if self.right {
            camera.position += diff.cross(previous.up) * speed as f32 / 100000.0;
        }
        if self.up {
            camera.position.y += speed as f32 / 100000.0;
        }
        if self.down {
            camera.position.y -= speed as f32 / 100000.0;
        }
        camera.yaw += self.mouse_delta.0 * sensitivity as f64;
        camera.pitch -= self.mouse_delta.1 * sensitivity as f64;

        camera.pitch = camera.pitch.clamp(-1.57, 1.57);

        let direction = cgmath::Vector3::new(
            (camera.yaw.cos() * camera.pitch.cos()) as f32,
            camera.pitch.sin() as f32,
            (camera.yaw.sin() * camera.pitch.cos()) as f32,
        );

        self.mouse_delta = (0.0, 0.0);

        camera.target = camera.position + direction.normalize();
    }
}
