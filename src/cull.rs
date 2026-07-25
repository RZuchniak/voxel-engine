use std::collections::VecDeque;

use cgmath::{InnerSpace, Matrix, Matrix4, Vector3, Vector4};

use crate::visibility::{Facing, VisibilitySet, FACINGS};
use crate::world::SECTION_COUNT;

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

/// A chunk section: horizontal chunk coordinate plus an index into its 24 vertical slices.
pub type SectionKey = ((i32, i32), usize);

#[derive(Clone, Copy)]
struct Node {
    key: SectionKey,
    /// The face of `key` we arrived through. `None` for the section holding the camera.
    entered_by: Option<Facing>,
    /// Directions taken so far along this path, as a bitmask over [`Facing::index`].
    steps_taken: u8,
}

/// Minecraft's cave culling: walk outward from the camera's section, stepping into a
/// neighbour only when the section you are in actually lets you see from the face you came in
/// by to the face you would leave by.
///
/// The effect is that standing on the surface draws the surface, because the sections below
/// are behind solid rock and nothing connects to them — no depth heuristic required. Sections
/// that are merely *near* are not drawn; sections that are *reachable by a sightline* are.
///
/// Reused across frames so the scratch buffers stay allocated.
/// Per-section traversal state, in the flat grid.
const UNSEEN: u8 = 0;
const REACHED: u8 = 1;
/// Failed the caller's frustum/distance test. Recorded so a section neighbouring several
/// reached ones is tested once rather than up to six times — the test is the most expensive
/// thing in the walk, and the frontier is mostly shared edges.
const REJECTED: u8 = 2;

pub struct SectionGraph {
    /// Flat `(2r+1) × SECTION_COUNT × (2r+1)` grid over the traversal volume.
    state: Vec<u8>,
    queue: VecDeque<Node>,
    radius: i32,
    origin_chunk: (i32, i32),
}

impl SectionGraph {
    pub fn new() -> Self {
        Self {
            state: Vec::new(),
            queue: VecDeque::new(),
            radius: -1,
            origin_chunk: (0, 0),
        }
    }

    /// Did the last [`walk`](Self::walk) reach this section? Sections outside the walked
    /// volume answer `false`.
    pub fn reached(&self, key: SectionKey) -> bool {
        self.slot(self.origin_chunk, key)
            .is_some_and(|slot| self.state[slot] == REACHED)
    }

    fn slot(&self, origin: (i32, i32), key: SectionKey) -> Option<usize> {
        let dx = key.0.0 - origin.0 + self.radius;
        let dz = key.0.1 - origin.1 + self.radius;
        let span = 2 * self.radius + 1;
        if dx < 0 || dz < 0 || dx >= span || dz >= span || key.1 >= SECTION_COUNT {
            return None;
        }
        Some(((dx * span + dz) as usize) * SECTION_COUNT + key.1)
    }

    /// Visits every section reachable from the camera's, in BFS order.
    ///
    /// `visibility` supplies a section's [`VisibilitySet`] — return [`VisibilitySet::EMPTY`]
    /// for sections whose blocks are not loaded, so an unknown section never blocks a sightline
    /// that really exists. `admits` is the caller's frustum and distance test; a section that
    /// fails it is neither drawn nor traversed, which is safe because anything visible through
    /// it would be further away and also outside the frustum.
    pub fn walk(
        &mut self,
        origin: SectionKey,
        radius: i32,
        visibility: impl Fn(SectionKey) -> VisibilitySet,
        admits: impl Fn(SectionKey) -> bool,
        mut visit: impl FnMut(SectionKey),
    ) {
        let span = (2 * radius + 1) as usize;
        let needed = span * span * SECTION_COUNT;
        if self.radius != radius {
            self.state = vec![UNSEEN; needed];
            self.radius = radius;
        } else {
            self.state.fill(UNSEEN);
        }
        self.queue.clear();

        let origin_chunk = origin.0;
        self.origin_chunk = origin_chunk;
        if origin.1 >= SECTION_COUNT {
            return;
        }
        if let Some(slot) = self.slot(origin_chunk, origin) {
            self.state[slot] = REACHED;
        }
        // The camera's own section is always drawn and always traversable in every direction:
        // there is no face we entered it by, and the camera may well be inside solid rock.
        self.queue.push_back(Node {
            key: origin,
            entered_by: None,
            steps_taken: 0,
        });

        while let Some(node) = self.queue.pop_front() {
            visit(node.key);
            let set = visibility(node.key);

            for step in FACINGS {
                // Leaving through the face we came in by is walking back down the path.
                if node.entered_by == Some(step) {
                    continue;
                }
                // Never undo a direction already taken. Without this the search wanders
                // sideways and back, and the frontier stops shrinking.
                if node.steps_taken & (1 << step.opposite().index()) != 0 {
                    continue;
                }
                // The camera's section is entered from nowhere, so every direction is open.
                if let Some(entry) = node.entered_by {
                    if !set.connects(entry, step) {
                        continue;
                    }
                }

                let (dx, dy, dz) = step.offset();
                let section = node.key.1 as i32 + dy;
                if section < 0 || section >= SECTION_COUNT as i32 {
                    continue;
                }
                let neighbour: SectionKey =
                    ((node.key.0.0 + dx, node.key.0.1 + dz), section as usize);

                let Some(slot) = self.slot(origin_chunk, neighbour) else {
                    continue;
                };
                if self.state[slot] != UNSEEN {
                    continue;
                }
                if !admits(neighbour) {
                    self.state[slot] = REJECTED;
                    continue;
                }
                self.state[slot] = REACHED;
                self.queue.push_back(Node {
                    key: neighbour,
                    entered_by: Some(step.opposite()),
                    steps_taken: node.steps_taken | (1 << step.index()),
                });
            }
        }
    }
}

impl Default for SectionGraph {
    fn default() -> Self {
        Self::new()
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

    /// Collect the sections a walk reaches, over a world described by a visibility closure.
    fn walked(
        origin: SectionKey,
        radius: i32,
        visibility: impl Fn(SectionKey) -> VisibilitySet,
    ) -> std::collections::HashSet<SectionKey> {
        let mut graph = SectionGraph::new();
        let mut seen = std::collections::HashSet::new();
        graph.walk(origin, radius, visibility, |_| true, |key| {
            seen.insert(key);
        });
        seen
    }

    #[test]
    fn open_air_reaches_the_whole_volume() {
        let radius = 3;
        let seen = walked(((0, 0), 12), radius, |_| VisibilitySet::EMPTY);
        // (2r+1)^2 chunks x 24 sections, all mutually connected.
        assert_eq!(seen.len(), ((2 * radius + 1) * (2 * radius + 1)) as usize * SECTION_COUNT);
    }

    #[test]
    fn solid_world_stops_at_the_walls_of_the_camera_section() {
        // Sealed in rock: you still see the six walls around you, and those faces live in the
        // six neighbouring sections, so those are drawn. Nothing beyond them is.
        let origin = ((0, 0), 12);
        let seen = walked(origin, 3, |key| {
            if key == origin {
                VisibilitySet::EMPTY
            } else {
                VisibilitySet::OPAQUE
            }
        });
        assert_eq!(seen.len(), 7, "the camera's section plus its six walls");
        assert!(seen.contains(&origin));
        for face in FACINGS {
            let (dx, dy, dz) = face.offset();
            let neighbour = ((dx, dz), (12 + dy) as usize);
            assert!(seen.contains(&neighbour), "wall {face:?} should be drawn");
        }
        assert!(!seen.contains(&((2, 0), 12)), "nothing past the walls");
    }

    #[test]
    fn buried_sections_are_skipped_but_the_surface_is_not() {
        // A world split like the real one: everything at or above section 12 is open sky,
        // everything below is solid. Standing on the surface must draw the surface layer and
        // nothing underneath — this is the whole point of the graph.
        let surface = 12usize;
        let radius = 4;
        let seen = walked(((0, 0), surface), radius, |key| {
            if key.1 >= surface {
                VisibilitySet::EMPTY
            } else {
                VisibilitySet::OPAQUE
            }
        });

        // One section of overshoot is correct, not a leak: the rock directly beneath you is
        // the ground you are standing on, and its top face is visible.
        assert!(
            seen.iter().all(|key| key.1 >= surface - 1),
            "traversal reached more than one section into solid rock"
        );
        assert!(
            seen.contains(&((radius, radius), surface)),
            "the far corner of the surface layer must still be reached"
        );
        // Sections below stay unvisited even though they are well inside the radius.
        assert!(!seen.contains(&((1, 0), 4)));
    }

    #[test]
    fn a_tunnel_is_followed_but_the_rock_around_it_is_not() {
        // One section-high corridor running along +X at section 8, solid elsewhere. The walk
        // should march down the corridor and never step off it.
        let corridor = 8usize;
        let mut only_x = [0u8; 6];
        let _ = &mut only_x;
        let seen = walked(((0, 0), corridor), 5, |key| {
            if key.1 == corridor && key.0.1 == 0 {
                // Open along X only.
                let mut section = crate::world::Section::new();
                for x in 0..crate::world::SECTION_SIZE {
                    section.set_block(x, 8, 8, crate::block::BlockId::AIR);
                }
                for x in 0..crate::world::SECTION_SIZE {
                    for y in 0..crate::world::SECTION_SIZE {
                        for z in 0..crate::world::SECTION_SIZE {
                            if !(y == 8 && z == 8) {
                                section.set_block(x, y, z, crate::block::BlockId::STONE);
                            }
                        }
                    }
                }
                VisibilitySet::from_section(&section)
            } else {
                VisibilitySet::OPAQUE
            }
        });

        assert!(seen.contains(&((5, 0), corridor)), "should reach the end of the corridor");
        assert!(seen.contains(&((-5, 0), corridor)), "and the other end");
        assert!(!seen.contains(&((1, 1), corridor)), "must not leave the corridor sideways");
        assert!(!seen.contains(&((1, 0), corridor + 1)), "nor vertically");
    }

    #[test]
    fn the_frustum_test_prunes_whole_branches() {
        // Open world, but the caller refuses everything with negative x. Nothing beyond the
        // cut should be visited, including sections reachable only by going around it.
        let mut graph = SectionGraph::new();
        let mut seen = std::collections::HashSet::new();
        graph.walk(
            ((0, 0), 12),
            3,
            |_| VisibilitySet::EMPTY,
            |key| key.0.0 >= 0,
            |key| {
                seen.insert(key);
            },
        );
        assert!(seen.iter().all(|key| key.0.0 >= 0));
        assert!(seen.contains(&((3, 0), 12)));
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
