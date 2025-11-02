use cgmath::{InnerSpace, Matrix4, Vector3};

#[derive(Clone)]
pub struct Camera {
    position: cgmath::Point3<f32>,
    target: cgmath::Point3<f32>,
    up: cgmath::Vector3<f32>,
    aspect_ratio: f32,
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
            position: (1.5, 0.5, 0.5).into(),
            target: (0.5, 0.5, 0.5).into(),
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
    pub speed: f32,
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
    pub fn new(speed: f32, sensitivity: f32) -> Self {
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

    pub fn update(&mut self, delta_time: f32, camera: &mut Camera) {
        let previous = camera.clone();
        let speed = self.speed * delta_time;
        let sensitivity = self.sensitivity * delta_time;
        let diff = previous.target - previous.position;

        if self.forward {
            camera.position += diff * speed;
            self.forward = false;
        }
        if self.backward {
            camera.position -= diff * speed;
            self.backward = false;
        }
        if self.left {
            camera.position += diff.cross(previous.up) * speed;
            self.left = false;
        }
        if self.right {
            camera.position -= diff.cross(previous.up) * speed;
            self.right = false;
        }
        if self.up {
            camera.position.y += speed * speed;
            self.up = false;
        }
        if self.down {
            camera.position.y -= speed * speed;
            self.down = false;
        }
        camera.yaw += self.mouse_delta.0 * sensitivity as f64;
        camera.pitch -= self.mouse_delta.1 * sensitivity as f64;

        camera.pitch = camera.pitch.clamp(-89.0, 89.0);

        let direction = cgmath::Vector3::new(
            (camera.yaw.cos() * camera.pitch.cos()) as f32,
            (camera.pitch.sin() as f32) as f32,
            (camera.yaw.sin() * camera.pitch.cos()) as f32,
        );

        self.mouse_delta = (0.0, 0.0);

        camera.target = camera.position + direction.normalize();
    }
}
