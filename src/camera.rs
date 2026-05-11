use cgmath::InnerSpace;

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
    pub fn position(&self) -> cgmath::Point3<f32> {
        self.position
    }

    pub fn forward(&self) -> cgmath::Vector3<f32> {
        (self.target - self.position).normalize()
    }

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

    pub fn set_far_plane(&mut self, zfar: f32) {
        // Keep a small minimum margin above near plane.
        self.zfar = zfar.max(self.znear + 1.0);
    }

    pub fn new(aspect_ratio: f32, fov: f32, znear: f32, zfar: f32) -> Self {
        let position: cgmath::Point3<f32> = (8.0, 100.0, 8.0).into();
        let target: cgmath::Point3<f32> = (0.0, 76.0, 0.0).into();
        let look = (target - position).normalize();
        let yaw = look.z.atan2(look.x) as f64;
        let pitch = look.y.asin() as f64;
        Self {
            position,
            target,
            up: cgmath::Vector3::unit_y(),
            aspect_ratio,
            fov,
            znear,
            zfar,
            pitch,
            yaw,
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

    pub fn update(&mut self, delta_time: u128, camera: &mut Camera) {
        let previous = camera.clone();
        let delta_seconds = (delta_time as f32 / 1_000_000.0).clamp(0.0, 0.1);
        // Minecraft-style movement: forward/backward ignores pitch (no Y drift).
        let horizontal_forward = cgmath::Vector3::new(
            previous.yaw.cos() as f32,
            0.0,
            previous.yaw.sin() as f32,
        )
        .normalize();
        let right = horizontal_forward.cross(previous.up).normalize();
        let move_step = self.speed * delta_seconds;

        if self.forward {
            camera.position += horizontal_forward * move_step;
        }
        if self.backward {
            camera.position -= horizontal_forward * move_step;
        }
        if self.left {
            camera.position -= right * move_step;
        }
        if self.right {
            camera.position += right * move_step;
        }
        if self.up {
            camera.position.y += move_step;
        }
        if self.down {
            camera.position.y -= move_step;
        }
        camera.yaw += self.mouse_delta.0 * self.sensitivity as f64;
        camera.pitch -= self.mouse_delta.1 * self.sensitivity as f64;

        camera.pitch = camera.pitch.clamp(-1.57, 1.57);

        let direction = cgmath::Vector3::new(
            (camera.yaw.cos() * camera.pitch.cos()) as f32,
            camera.pitch.sin() as f32,
            (camera.yaw.sin() * camera.pitch.cos()) as f32,
        );

        self.mouse_delta = (0.0, 0.0);

        camera.target = camera.position + direction.normalize();
    }

    /// Mouse look only (used while the world is bootstrapping — no WASD / fly movement).
    pub fn update_look_only(&mut self, camera: &mut Camera) {
        camera.yaw += self.mouse_delta.0 * self.sensitivity as f64;
        camera.pitch -= self.mouse_delta.1 * self.sensitivity as f64;

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
