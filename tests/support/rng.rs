//! splitmix64, the integration tests' one source of randomness, seeded per run (Steele, Lea and
//! Flood, "Fast splittable pseudorandom number generators", OOPSLA 2014). `tests/mcp_support`
//! and `tests/mutants_verdict_journey.rs` include it by `#[path]`.

/// The generator's state; one seed, one sequence.
pub(crate) struct Rng(pub u64);

impl Rng {
    /// The next 64 random bits.
    pub(crate) fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut mixed = self.0;
        mixed = (mixed ^ mixed.wrapping_shr(30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        mixed = (mixed ^ mixed.wrapping_shr(27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        mixed ^ mixed.wrapping_shr(31)
    }

    /// A value below `bound`, by multiply-shift: no modulo, no division.
    pub(crate) fn below(&mut self, bound: u32) -> u32 {
        let [_, _, _, _, high_0, high_1, high_2, high_3] = self.next().to_le_bytes();
        let high = u32::from_le_bytes([high_0, high_1, high_2, high_3]);
        let [low_0, low_1, low_2, low_3, ..] = u64::from(high)
            .wrapping_mul(u64::from(bound))
            .wrapping_shr(32)
            .to_le_bytes();
        u32::from_le_bytes([low_0, low_1, low_2, low_3])
    }

    /// `below`, as a length or index.
    pub(crate) fn index(&mut self, bound: u32) -> Result<usize, std::num::TryFromIntError> {
        usize::try_from(self.below(bound))
    }
}
