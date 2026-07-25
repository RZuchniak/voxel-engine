//! The 6-dimensional climate space biomes are looked up in — `net.minecraft.world.level
//! .biome.Climate`.
//!
//! Every biome owns one or more axis-aligned boxes in (temperature, humidity,
//! continentalness, erosion, depth, weirdness) space, plus a scalar `offset` that acts as a
//! constant distance penalty. A position samples those six coordinates and takes the biome
//! whose box is *nearest* — nearest meaning sum of squared per-axis distances, where being
//! inside an axis' range contributes 0.
//!
//! Coordinates are **quantised to fixed point** (`×10000`, truncated) and compared as
//! integers, so the lookup is exact and reproducible.
//!
//! ## On the search
//!
//! Vanilla accelerates this with an R-tree (`Climate.RTree`), but the tree is a pure
//! index — Mojang ships `ParameterList.findValueBruteForce` as its `@VisibleForTesting`
//! reference, and `RTree.search` is exact branch-and-bound over the same metric. So the
//! linear scan here returns the same biome. (One caveat: on an exact `fitness` tie the two
//! can disagree, since brute force keeps the first in list order and the tree keeps
//! whichever candidate it was carrying. Vanilla's boxes barely overlap, so ties are rare —
//! and `examples/parity_biomes` checks the result against real chunks regardless.)

/// `Climate.quantizeCoord` — fixed-point encoding. The multiply happens in `f32` and the
/// cast truncates toward zero, both of which matter for exactness at box edges.
#[inline]
pub fn quantize(coord: f32) -> i64 {
    (coord * 10000.0f32) as i64
}

/// `Climate.Parameter` — an inclusive quantised range on one axis.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Parameter {
    pub min: i64,
    pub max: i64,
}

impl Parameter {
    /// `Parameter.span(float, float)`.
    pub const fn raw(min: i64, max: i64) -> Self {
        Self { min, max }
    }

    /// `Parameter.span(float min, float max)`.
    pub fn span(min: f32, max: f32) -> Self {
        Self { min: quantize(min), max: quantize(max) }
    }

    /// `Parameter.point(float)` — a zero-width range.
    pub fn point(v: f32) -> Self {
        Self::span(v, v)
    }

    /// `Parameter.span(Parameter, Parameter)` — the hull from one range's min to another's max.
    pub fn hull(a: Parameter, b: Parameter) -> Self {
        Self { min: a.min, max: b.max }
    }

    /// `Parameter.distance(long)` — 0 inside the range, else the gap to the nearer edge.
    #[inline]
    pub fn distance(self, target: i64) -> i64 {
        let above = target - self.max;
        let below = self.min - target;
        if above > 0 {
            above
        } else {
            below.max(0)
        }
    }
}

/// `Climate.ParameterPoint` — one biome's box, plus its constant `offset` penalty.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ParameterPoint {
    pub temperature: Parameter,
    pub humidity: Parameter,
    pub continentalness: Parameter,
    pub erosion: Parameter,
    pub depth: Parameter,
    pub weirdness: Parameter,
    pub offset: i64,
}

impl ParameterPoint {
    /// `ParameterPoint.fitness(TargetPoint)` — squared distance summed over the six axes,
    /// with `offset²` as a seventh term. Lower wins.
    #[inline]
    pub fn fitness(&self, t: &TargetPoint) -> i64 {
        let sq = |v: i64| v * v;
        sq(self.temperature.distance(t.temperature))
            + sq(self.humidity.distance(t.humidity))
            + sq(self.continentalness.distance(t.continentalness))
            + sq(self.erosion.distance(t.erosion))
            + sq(self.depth.distance(t.depth))
            + sq(self.weirdness.distance(t.weirdness))
            + sq(self.offset)
    }

    /// [`Self::fitness`], abandoned as soon as the running total reaches `limit`.
    ///
    /// Every term is non-negative, so a partial sum is a lower bound on the whole — once it
    /// reaches the best fitness seen so far, this entry cannot win and the rest of the terms
    /// are wasted work. Returns `None` in that case. Selection is unchanged; this is purely
    /// a speedup for the linear scan, which runs 7594 times per lookup.
    ///
    /// Axis order is deliberate: `depth` first because vanilla's boxes are pinned to a few
    /// discrete depths (0, 1, 0.2..0.9, 1.1) and so reject hardest, then the two axes that
    /// carve the map into the largest regions.
    #[inline]
    fn fitness_below(&self, t: &TargetPoint, limit: i64) -> Option<i64> {
        let sq = |v: i64| v * v;
        let mut sum = sq(self.depth.distance(t.depth));
        if sum >= limit {
            return None;
        }
        sum += sq(self.continentalness.distance(t.continentalness));
        if sum >= limit {
            return None;
        }
        sum += sq(self.erosion.distance(t.erosion));
        if sum >= limit {
            return None;
        }
        sum += sq(self.weirdness.distance(t.weirdness));
        if sum >= limit {
            return None;
        }
        sum += sq(self.temperature.distance(t.temperature));
        if sum >= limit {
            return None;
        }
        sum += sq(self.humidity.distance(t.humidity)) + sq(self.offset);
        if sum >= limit { None } else { Some(sum) }
    }
}

/// `Climate.TargetPoint` — the sampled climate at one position.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TargetPoint {
    pub temperature: i64,
    pub humidity: i64,
    pub continentalness: i64,
    pub erosion: i64,
    pub depth: i64,
    pub weirdness: i64,
}

impl TargetPoint {
    /// `Climate.target(...)` — note every coordinate is narrowed to `f32` before quantising,
    /// exactly as `Climate.Sampler.sample` does.
    pub fn new(
        temperature: f64,
        humidity: f64,
        continentalness: f64,
        erosion: f64,
        depth: f64,
        weirdness: f64,
    ) -> Self {
        Self {
            temperature: quantize(temperature as f32),
            humidity: quantize(humidity as f32),
            continentalness: quantize(continentalness as f32),
            erosion: quantize(erosion as f32),
            depth: quantize(depth as f32),
            weirdness: quantize(weirdness as f32),
        }
    }
}

impl ParameterPoint {
    /// `ParameterPoint.parameterSpace()` — the box as 7 axes, with `offset` as a zero-width
    /// seventh. The target's seventh coordinate is always 0, so that axis contributes
    /// exactly `|offset|`, which is how the offset penalty falls out of the generic metric.
    fn parameter_space(&self) -> [Parameter; 7] {
        [
            self.temperature,
            self.humidity,
            self.continentalness,
            self.erosion,
            self.depth,
            self.weirdness,
            Parameter { min: self.offset, max: self.offset },
        ]
    }
}

impl TargetPoint {
    /// `TargetPoint.toParameterArray()`.
    fn to_array(self) -> [i64; 7] {
        [
            self.temperature,
            self.humidity,
            self.continentalness,
            self.erosion,
            self.depth,
            self.weirdness,
            0,
        ]
    }
}

/// `Climate.RTree.Node` — a leaf carries an index into [`ParameterList::entries`]; a subtree
/// carries the bounding box of its children.
enum Node {
    Leaf { space: [Parameter; 7], entry: usize },
    SubTree { space: [Parameter; 7], children: Vec<Node> },
}

impl Node {
    fn space(&self) -> &[Parameter; 7] {
        match self {
            Node::Leaf { space, .. } | Node::SubTree { space, .. } => space,
        }
    }

    /// `Node.distance` — squared distance from the target to this node's box.
    #[inline]
    fn distance(&self, target: &[i64; 7]) -> i64 {
        let space = self.space();
        let mut sum = 0;
        for i in 0..7 {
            let d = space[i].distance(target[i]);
            sum += d * d;
        }
        sum
    }

    /// The axis midpoint the tree sorts on.
    fn center(&self, axis: usize) -> i64 {
        let p = self.space()[axis];
        (p.min + p.max) / 2
    }

    /// `SubTree.search` — branch and bound. A child whose *box* is already further than the
    /// best leaf found so far cannot contain a better leaf, so its subtree is skipped
    /// entirely. That is what makes this exact rather than approximate.
    fn search(&self, target: &[i64; 7], candidate: Option<(usize, i64)>) -> (usize, i64) {
        match self {
            Node::Leaf { entry, .. } => (*entry, self.distance(target)),
            Node::SubTree { children, .. } => {
                let (mut best_entry, mut best_distance) = match candidate {
                    Some(c) => c,
                    None => (usize::MAX, i64::MAX),
                };
                for child in children {
                    let child_distance = child.distance(target);
                    if best_distance > child_distance {
                        let found = child.search(
                            target,
                            (best_entry != usize::MAX).then_some((best_entry, best_distance)),
                        );
                        if best_distance > found.1 {
                            best_distance = found.1;
                            best_entry = found.0;
                        }
                    }
                }
                (best_entry, best_distance)
            }
        }
    }
}

/// `Climate.ParameterList` — the searchable set of (box → value) pairs, indexed by an R-tree.
pub struct ParameterList<T> {
    pub entries: Vec<(ParameterPoint, T)>,
    root: Node,
}

impl<T> ParameterList<T> {
    pub fn new(entries: Vec<(ParameterPoint, T)>) -> Self {
        assert!(!entries.is_empty(), "need at least one value to search");
        let leaves: Vec<Node> = entries
            .iter()
            .enumerate()
            .map(|(i, (point, _))| Node::Leaf { space: point.parameter_space(), entry: i })
            .collect();
        let root = build(leaves);
        Self { entries, root }
    }

    /// The nearest box's value, via the R-tree (`RTree.search`).
    ///
    /// Vanilla seeds the search with the *previous* query's leaf (a `ThreadLocal`), which
    /// biases tie-breaking toward whatever was looked up last. That makes its result depend
    /// on query order, which this port does not reproduce — it starts from no candidate.
    /// Only exact `fitness` ties can differ, and vanilla's boxes barely overlap.
    pub fn find(&self, target: &TargetPoint) -> &T {
        let (entry, _) = self.root.search(&target.to_array(), None);
        &self.entries[entry].1
    }

    /// `findValueBruteForce` — the linear reference. Kept because it is Mojang's own
    /// `@VisibleForTesting` oracle for the tree, and `rtree_agrees_with_brute_force` holds
    /// the two against each other.
    pub fn find_brute_force(&self, target: &TargetPoint) -> &T {
        let mut best = &self.entries[0];
        let mut best_fitness = best.0.fitness(target);
        for entry in &self.entries[1..] {
            // `fitness_below` returns `None` exactly when `fitness >= best_fitness`, which is
            // the same test the plain form does — so ties still keep the earlier entry.
            if let Some(fitness) = entry.0.fitness_below(target, best_fitness) {
                best_fitness = fitness;
                best = entry;
            }
        }
        &best.1
    }
}

/// Number of children per node vanilla packs a bucket to (`CHILDREN_PER_NODE`).
const CHILDREN_PER_NODE: usize = 6;

/// `RTree.build` — recursively group nodes into subtrees, choosing at each level the axis
/// whose split yields the tightest bounding boxes.
fn build(mut children: Vec<Node>) -> Node {
    assert!(!children.is_empty(), "need at least one child to build a node");
    if children.len() == 1 {
        return children.pop().expect("length checked");
    }
    if children.len() <= CHILDREN_PER_NODE {
        // Small groups just get ordered by total magnitude across all axes.
        children.sort_by_key(|node| {
            (0..7).map(|axis| node.center(axis).abs()).sum::<i64>()
        });
        return sub_tree(children);
    }

    // Try splitting on each axis; keep whichever gives the least total box perimeter.
    let mut min_cost = i64::MAX;
    let mut min_axis = 0usize;
    let mut min_buckets: Vec<Vec<Node>> = Vec::new();
    for axis in 0..7 {
        sort_nodes(&mut children, axis, false);
        let buckets = bucketize(&children);
        let cost: i64 = buckets.iter().map(|b| cost(&bounding_space(b))).sum();
        if min_cost > cost {
            min_cost = cost;
            min_axis = axis;
            min_buckets = buckets;
        }
    }

    // Re-sort the winning buckets by |center| on the winning axis, then recurse into each.
    let mut bucket_nodes: Vec<Node> = min_buckets.into_iter().map(sub_tree).collect();
    sort_nodes(&mut bucket_nodes, min_axis, true);
    let rebuilt = bucket_nodes
        .into_iter()
        .map(|bucket| match bucket {
            Node::SubTree { children, .. } => build(children),
            leaf => leaf,
        })
        .collect();
    sub_tree(rebuilt)
}

fn sub_tree(children: Vec<Node>) -> Node {
    Node::SubTree { space: bounding_space(&children), children }
}

/// `RTree.sort` — order by one axis, breaking ties with the remaining axes in rotation.
fn sort_nodes(nodes: &mut [Node], axis: usize, absolute: bool) {
    let key = |node: &Node| -> [i64; 7] {
        let mut out = [0i64; 7];
        for d in 0..7 {
            let c = node.center((axis + d) % 7);
            out[d] = if absolute { c.abs() } else { c };
        }
        out
    };
    nodes.sort_by_key(key);
}

/// `RTree.bucketize` — split into runs of `6^floor(log6(n - 0.01))`.
fn bucketize(nodes: &[Node]) -> Vec<Vec<Node>>
where
{
    let n = nodes.len();
    let expected = 6f64
        .powf(((n as f64 - 0.01).ln() / 6f64.ln()).floor())
        as usize;
    let expected = expected.max(1);
    let mut buckets = Vec::new();
    let mut current: Vec<Node> = Vec::new();
    for node in nodes {
        current.push(clone_node(node));
        if current.len() >= expected {
            buckets.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        buckets.push(current);
    }
    buckets
}

/// `bucketize` is called once per axis on the same slice, so nodes must be duplicated rather
/// than moved. Only the winning axis' buckets survive.
fn clone_node(node: &Node) -> Node {
    match node {
        Node::Leaf { space, entry } => Node::Leaf { space: *space, entry: *entry },
        Node::SubTree { space, children } => {
            Node::SubTree { space: *space, children: children.iter().map(clone_node).collect() }
        }
    }
}

/// `RTree.cost` — total width across all axes; smaller means a tighter box.
fn cost(space: &[Parameter; 7]) -> i64 {
    space.iter().map(|p| (p.max - p.min).abs()).sum()
}

/// `RTree.buildParameterSpace` — the box enclosing every child.
fn bounding_space(children: &[Node]) -> [Parameter; 7] {
    let mut bounds = *children[0].space();
    for child in &children[1..] {
        let space = child.space();
        for d in 0..7 {
            bounds[d].min = bounds[d].min.min(space[d].min);
            bounds[d].max = bounds[d].max.max(space[d].max);
        }
    }
    bounds
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantize_truncates_toward_zero_in_f32() {
        assert_eq!(quantize(0.0), 0);
        assert_eq!(quantize(1.0), 10000);
        assert_eq!(quantize(-1.0), -10000);
        assert_eq!(quantize(-0.45), -4500);
        // 0.56666666_f32 * 10000 lands just under 5666.7; the cast truncates.
        assert_eq!(quantize(0.56666666), 5666);
    }

    #[test]
    fn distance_is_zero_inside_the_range() {
        let p = Parameter::span(-0.5, 0.5);
        assert_eq!(p.distance(0), 0);
        assert_eq!(p.distance(-5000), 0);
        assert_eq!(p.distance(5000), 0);
        assert_eq!(p.distance(6000), 1000);
        assert_eq!(p.distance(-6000), 1000);
    }

    /// The tree must return what the linear scan returns. Mojang ships
    /// `findValueBruteForce` as the tree's own test oracle, and this holds the two against
    /// each other over the real 7594-box overworld table across a spread of targets.
    #[test]
    fn rtree_agrees_with_brute_force() {
        let list = ParameterList::new(crate::mc::biome::overworld_biomes());
        // A deterministic spread over climate space, including out-of-range coordinates so
        // the "nearest box" path (not just "inside a box") gets exercised.
        let mut disagreements = 0;
        let mut checked = 0;
        for i in 0..12 {
            for j in 0..12 {
                let f = |n: i32| (n as f64 / 11.0) * 2.4 - 1.2;
                let target = TargetPoint::new(f(i), f(j), f((i + 5) % 12), f((j + 3) % 12), f(i % 3), f((i + j) % 12));
                checked += 1;
                if list.find(&target) != list.find_brute_force(&target) {
                    disagreements += 1;
                }
            }
        }
        assert!(checked > 100, "expected a decent sample, got {checked}");
        assert_eq!(disagreements, 0, "R-tree disagreed with brute force on {disagreements}/{checked}");
    }

    #[test]
    fn nearest_box_wins_and_offset_penalises() {
        let full = Parameter::span(-1.0, 1.0);
        let near = ParameterPoint {
            temperature: Parameter::span(-1.0, 0.0),
            humidity: full,
            continentalness: full,
            erosion: full,
            depth: full,
            weirdness: full,
            offset: 0,
        };
        // Same box, but a large offset penalty must lose to the unpenalised one.
        let penalised = ParameterPoint { offset: quantize(0.5), ..near };
        let list = ParameterList::new(vec![(near, "near"), (penalised, "penalised")]);
        let target = TargetPoint::new(-0.5, 0.0, 0.0, 0.0, 0.0, 0.0);
        assert_eq!(*list.find(&target), "near");
    }
}
