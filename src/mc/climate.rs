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

/// `Climate.ParameterList` — the searchable set of (box → value) pairs.
pub struct ParameterList<T> {
    pub entries: Vec<(ParameterPoint, T)>,
}

impl<T> ParameterList<T> {
    pub fn new(entries: Vec<(ParameterPoint, T)>) -> Self {
        assert!(!entries.is_empty(), "need at least one value to search");
        Self { entries }
    }

    /// `findValueBruteForce` — the nearest box's value. First entry wins ties.
    pub fn find(&self, target: &TargetPoint) -> &T {
        let mut best = &self.entries[0];
        let mut best_fitness = best.0.fitness(target);
        for entry in &self.entries[1..] {
            let fitness = entry.0.fitness(target);
            if fitness < best_fitness {
                best_fitness = fitness;
                best = entry;
            }
        }
        &best.1
    }
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
