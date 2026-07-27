use std::collections::VecDeque;

use cgmath::{InnerSpace, Matrix, Matrix4, Vector3, Vector4};

use crate::block::BlockId;
use crate::visibility::{Facing, VisibilitySet, FACINGS};
use crate::world::{MIN_SECTION_Y, SECTION_COUNT, SECTION_SIZE};

/// Which vertical section a world Y falls in, and whether it had to be clamped to reach one.
///
/// The world is only `-64..320`, so a noclipping camera can leave it in either direction. Clamping
/// is the only sane thing to do with the index, but the caller must know it happened — see
/// [`smart_cull_from_camera`].
pub fn camera_section_index(world_y: i32) -> (usize, bool) {
    let raw = world_y.div_euclid(SECTION_SIZE as i32) - MIN_SECTION_Y;
    let clamped = raw.clamp(0, SECTION_COUNT as i32 - 1);
    (clamped as usize, raw != clamped)
}

/// Should the walk apply vanilla's `smartCull` restrictions, given where the camera is?
///
/// Dropped when **the camera is inside an opaque full cube**: it sits in a cell no sightline leaves,
/// so the walk dies after the six solid neighbours and the world renders empty. Vanilla gates on the
/// same condition (`LevelRenderer.setupRender`: `isSolidRender` ⇒ `flag = false`). With it false the
/// walk degrades to a plain frustum flood fill, which the caller's `admits` test still bounds.
///
/// Note the camera being *outside the world* is a different problem with a different fix — it is
/// where the walk **starts** that is wrong there, not what it may step through. See
/// `SectionGraph::walk`'s `seed_whole_layer`.
pub fn smart_cull_from_camera(camera_block: BlockId) -> bool {
    !(camera_block.is_opaque() && camera_block.is_full_cube())
}

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
    ///
    /// `smart_cull` is vanilla's flag of the same name. Pass `false` when the camera is inside
    /// an opaque block: connectivity is then meaningless — the eye is in a cell no sightline
    /// leaves — and the walk degrades to a plain frustum flood fill. Without this a noclipping
    /// camera underground draws *nothing*, since the camera's section and all six of its
    /// neighbours are solid rock with no faces to emit. Vanilla does exactly this for
    /// spectators (`LevelRenderer.setupRender`: `isSolidRender` ⇒ `flag = false`).
    pub fn walk(
        &mut self,
        origin: SectionKey,
        radius: i32,
        smart_cull: bool,
        seed_whole_layer: bool,
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
        if seed_whole_layer {
            // The camera is outside the world vertically, so `origin` is a *clamped* section — the
            // bedrock slab below, or the top slice above. Seeding one section there cannot work:
            // from under the world the slab's nearby sections are steeply overhead and fail the
            // frustum, and a rejected section stops the flood from ever reaching the far sections
            // that genuinely are in view. The symptom is that the world only appears once you look
            // up sharply enough to bring the nearby sections into the frustum.
            //
            // Vanilla seeds the whole layer for exactly this case
            // (`SectionOcclusionGraph.initializeQueueForFullUpdate`). Connectivity still applies
            // from each seed onwards, so this widens where the walk *starts*, not what it admits.
            //
            // Each seed is entered through the face nearest the camera, travelling inward — from
            // below the world you enter the bedrock layer through its bottom and head up. That
            // matters: `entered_by: None` (what the camera's own section gets) waives the
            // connectivity test entirely, which would step out of every seed in all six directions
            // and pull in a whole extra layer of sections nothing can see.
            let travelling = if origin.1 == 0 {
                Facing::PosY
            } else {
                Facing::NegY
            };
            for dx in -radius..=radius {
                for dz in -radius..=radius {
                    let key: SectionKey =
                        ((origin_chunk.0 + dx, origin_chunk.1 + dz), origin.1);
                    let Some(slot) = self.slot(origin_chunk, key) else {
                        continue;
                    };
                    if self.state[slot] != UNSEEN {
                        continue;
                    }
                    if !admits(key) {
                        self.state[slot] = REJECTED;
                        continue;
                    }
                    self.state[slot] = REACHED;
                    self.queue.push_back(Node {
                        key,
                        entered_by: Some(travelling.opposite()),
                        steps_taken: 1 << travelling.opposite().index(),
                    });
                }
            }
        } else {
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
        }

        while let Some(node) = self.queue.pop_front() {
            visit(node.key);
            // Nothing reads connectivity when smart culling is off, and the lookup is a hash
            // probe per section per frame.
            let set = if smart_cull {
                visibility(node.key)
            } else {
                VisibilitySet::EMPTY
            };

            for step in FACINGS {
                if smart_cull {
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
        graph.walk(origin, radius, true, false, visibility, |_| true, |key| {
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
            true,
            false,
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
    fn a_camera_inside_rock_falls_back_to_the_frustum() {
        // The engine's camera noclips, so underground it is usually *inside stone*. Smart
        // culling then reaches the camera's section and its six solid neighbours and stops —
        // none of which have any geometry, so the screen goes empty. With `smart_cull` off the
        // walk must instead fill the whole admitted volume.
        let origin = ((0, 0), 4);
        let radius = 3;
        let solid_world = |_: SectionKey| VisibilitySet::OPAQUE;

        assert_eq!(
            walked(origin, radius, solid_world).len(),
            7,
            "smart culling: the camera's section plus its six walls"
        );

        let mut graph = SectionGraph::new();
        let mut seen = std::collections::HashSet::new();
        graph.walk(origin, radius, false, false, solid_world, |_| true, |key| {
            seen.insert(key);
        });
        assert_eq!(
            seen.len(),
            ((2 * radius + 1) * (2 * radius + 1)) as usize * SECTION_COUNT,
            "without smart culling every admitted section is reached, rock or not"
        );
    }

    /// Leaving the world vertically must be detected, so the caller can seed the whole layer.
    ///
    /// The camera block down in the void is **air**, so the inside-rock test cannot notice this —
    /// which is why it needs its own signal rather than being folded into `smart_cull`.
    #[test]
    fn leaving_the_world_vertically_is_detected() {
        assert!(
            !BlockId::AIR.is_opaque(),
            "the void reads as air, which is why the inside-rock test misses this"
        );
        assert!(smart_cull_from_camera(BlockId::AIR));
        assert!(!smart_cull_from_camera(BlockId::STONE));

        // Inside the world, nothing is clamped.
        assert_eq!(camera_section_index(100).1, false);
        assert_eq!(camera_section_index(-64).1, false, "lowest block in the world");
        assert_eq!(camera_section_index(319).1, false, "highest block in the world");

        // Outside it, in either direction, and clamped to the near layer.
        assert_eq!(camera_section_index(-65), (0, true), "clamps to the bedrock layer");
        assert_eq!(camera_section_index(-80), (0, true));
        assert_eq!(camera_section_index(320), (SECTION_COUNT - 1, true));
        assert_eq!(camera_section_index(400), (SECTION_COUNT - 1, true));
    }

    /// Below bedrock the walk must start from the whole layer, not the one clamped section.
    ///
    /// Reported as "below bedrock the bedrock only renders in my current chunk and the adjacent
    /// ones", and then more precisely: it appears only once you look up past roughly 30°. That angle
    /// is the tell. The slab is directly *overhead*, so its nearby sections fall outside a 45° fov
    /// and fail the frustum — and **a frustum-rejected section stops the flood from reaching the far
    /// sections behind it**, which are the ones actually in view at a shallow angle. Widening the
    /// seed is the fix; `smart_cull` is a different question and stays on.
    ///
    /// Modelled here with an `admits` that rejects everything near the origin, exactly as the
    /// frustum does when the slab is overhead, and solid rock everywhere so connectivity alone can
    /// never carry the walk outward.
    #[test]
    fn seeding_the_whole_layer_reaches_sections_the_origin_cannot() {
        let solid = |_: SectionKey| VisibilitySet::OPAQUE;
        // Only sections at least 2 chunks away in x are "in the frustum".
        let admits = |key: SectionKey| key.0.0.abs() >= 2;
        let far: SectionKey = ((3, 0), 0);

        let mut graph = SectionGraph::new();
        let mut seen = std::collections::HashSet::new();
        graph.walk(((0, 0), 0), 4, true, false, solid, admits, |key| {
            seen.insert(key);
        });
        assert!(
            !seen.contains(&far),
            "single-section seeding cannot escape the rejected ring — this is the bug"
        );

        let mut seen = std::collections::HashSet::new();
        graph.walk(((0, 0), 0), 4, true, true, solid, admits, |key| {
            seen.insert(key);
        });
        assert!(
            seen.contains(&far),
            "seeding the layer must reach the visible far sections"
        );
        assert!(graph.reached(far));
        // Still bounded by `admits` — seeding wider must not become "draw everything".
        assert!(
            seen.iter().all(|key| key.0.0.abs() >= 2),
            "the frustum test must still prune the seeds"
        );
        // And it seeds only the one layer; connectivity is solid, so nothing else is reachable.
        assert!(
            seen.iter().all(|key| key.1 == 0),
            "only the clamped layer is seeded"
        );
    }

    #[test]
    fn the_frustum_still_prunes_without_smart_culling() {
        // Dropping connectivity must not drop the caller's test — that is the only thing
        // bounding the flood fill when the camera is buried.
        let mut graph = SectionGraph::new();
        let mut seen = std::collections::HashSet::new();
        graph.walk(
            ((0, 0), 12),
            3,
            false,
            false,
            |_| VisibilitySet::OPAQUE,
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
