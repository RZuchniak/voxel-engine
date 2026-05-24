use cgmath::{InnerSpace, Matrix, Matrix4, Vector3, Vector4};

/// View-projection frustum for axis-aligned bounding box tests.
pub struct Frustum {
    planes: [Vector4<f32>; 6],
}

impl Frustum {
    /// Gribb–Hartmann extraction from a column-major view-projection matrix (clip = m * v).
    pub fn from_view_projection(m: &Matrix4<f32>) -> Self {
        // clip.x = row0·v, clip.w = row3·v — planes combine matrix rows, not columns.
        let r0 = m.row(0);
        let r1 = m.row(1);
        let r2 = m.row(2);
        let r3 = m.row(3);
        let mut planes = [
            r3 + r0, // left:  clip.x + clip.w >= 0
            r3 - r0, // right: clip.w - clip.x >= 0
            r3 + r1, // bottom
            r3 - r1, // top
            r3 + r2, // near
            r3 - r2, // far
        ];
        for plane in &mut planes {
            let n = Vector3::new(plane.x, plane.y, plane.z);
            let len = n.magnitude();
            if len > 1e-6 {
                *plane /= len;
            }
        }
        Self { planes }
    }

    pub fn intersects_aabb(&self, min: Vector3<f32>, max: Vector3<f32>) -> bool {
        for plane in &self.planes {
            let n = Vector3::new(plane.x, plane.y, plane.z);
            let mut corner = min;
            if n.x >= 0.0 {
                corner.x = max.x;
            }
            if n.y >= 0.0 {
                corner.y = max.y;
            }
            if n.z >= 0.0 {
                corner.z = max.z;
            }
            if plane.dot(corner.extend(1.0)) < 0.0 {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgmath::{Deg, Point3};

    fn camera_view_projection(position: Point3<f32>, yaw: f64, pitch: f64) -> Matrix4<f32> {
        let direction = Vector3::new(
            (yaw.cos() * pitch.cos()) as f32,
            pitch.sin() as f32,
            (yaw.sin() * pitch.cos()) as f32,
        );
        let target = position + direction;
        let view = Matrix4::look_at_rh(position, target, Vector3::unit_y());
        let proj = cgmath::perspective(Deg(45.0), 16.0 / 9.0, 0.1, 1200.0);
        proj * view
    }

    fn chunk_aabb(chunk_x: i32, chunk_z: i32, min_y: f32, max_y: f32) -> (Vector3<f32>, Vector3<f32>) {
        const SECTION_SIZE: f32 = 16.0;
        let min = Vector3::new(
            chunk_x as f32 * SECTION_SIZE,
            min_y,
            chunk_z as f32 * SECTION_SIZE,
        );
        let max = Vector3::new(
            min.x + SECTION_SIZE,
            max_y,
            min.z + SECTION_SIZE,
        );
        (min, max)
    }

    fn clip_visible(vp: &Matrix4<f32>, world: Vector3<f32>) -> bool {
        let clip = vp * world.extend(1.0);
        if clip.w <= 0.0 {
            return false;
        }
        let ndc = clip / clip.w;
        ndc.x.abs() <= 1.0 && ndc.y.abs() <= 1.0 && ndc.z >= -1.0 && ndc.z <= 1.0
    }

    #[test]
    fn flanking_chunks_match_clip_space_visibility() {
        let position = Point3::new(80.0, 100.0, 100.0);
        let vp = camera_view_projection(position, 0.0, 0.0);
        let frustum = Frustum::from_view_projection(&vp);

        let (min_r, max_r) = chunk_aabb(7, 6, 92.0, 108.0);
        let (min_l, max_l) = chunk_aabb(7, 5, 92.0, 108.0);
        let center_r = (min_r + max_r) * 0.5;
        let center_l = (min_l + max_l) * 0.5;

        assert!(clip_visible(&vp, center_r));
        assert!(clip_visible(&vp, center_l));
        assert!(frustum.intersects_aabb(min_r, max_r));
        assert!(frustum.intersects_aabb(min_l, max_l));
    }

    #[test]
    fn tall_surface_chunk_stays_visible_when_looking_horizontally() {
        let position = Point3::new(80.0, 100.0, 100.0);
        let vp = camera_view_projection(position, 0.0, 0.0);
        let frustum = Frustum::from_view_projection(&vp);

        // Typical chunk bounds span many sections vertically even when only surface is meshed.
        let (min, max) = chunk_aabb(7, 6, -64.0, 320.0);
        let eye_level = Vector3::new((min.x + max.x) * 0.5, position.y, (min.z + max.z) * 0.5);
        assert!(clip_visible(&vp, eye_level));
        assert!(frustum.intersects_aabb(min, max));
    }
}
