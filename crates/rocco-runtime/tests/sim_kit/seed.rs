//! The one seed step every seeded simulation shares. Included with `#[path]`
//! only by the suites that evolve a seed, so no other binary carries it.

/// Wrapping seed evolution: multiply, add, then rotate left. Each suite passes
/// its own constants, so each keeps its own seed stream while the arithmetic
/// lives in exactly one place.
pub fn step(seed: &mut u64, multiplier: u64, addend: u64, rotation: u32) -> u64 {
    *seed = seed.wrapping_mul(multiplier).wrapping_add(addend);
    seed.rotate_left(rotation)
}
