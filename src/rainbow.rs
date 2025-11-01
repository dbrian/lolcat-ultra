use crate::color::Color;

// Table configuration
const TABLE_SIZE: usize = 2048;
const MASK: usize = TABLE_SIZE - 1; // For fast power-of-2 wrapping

// Include the pre-computed tables generated at build time
include!(concat!(env!("OUT_DIR"), "/rainbow_tables.rs"));

/// Pre-computed rainbow color lookup table for performance
pub struct RainbowLookup {
    scale: f64,
}

impl RainbowLookup {
    #[must_use]
    pub fn new(frequency: f64) -> Self {
        debug_assert!(
            frequency.is_finite() && frequency > 0.0,
            "Frequency must be finite and positive"
        );

        // Tighter math: scale = TABLE_SIZE * frequency / TAU
        let scale = (TABLE_SIZE as f64) * (frequency / std::f64::consts::TAU);

        Self { scale }
    }

    /// Get the rainbow color at a given position.
    #[inline(always)]
    #[must_use]
    pub fn get_color(&self, position: f64) -> Color {
        RAINBOW_TABLE[self.index_from_position(position)]
    }

    /// Get the rainbow color and table index at a given position.
    /// Returns (`Color`, `table_index`) for use with ANSI sequence caching.
    #[inline(always)]
    #[must_use]
    pub fn get_color_with_index(&self, position: f64) -> (Color, usize) {
        let idx = self.index_from_position(position);
        (RAINBOW_TABLE[idx], idx)
    }

    /// Get pre-built `TrueColor` ANSI sequence for a table index
    #[inline(always)]
    #[must_use]
    pub fn get_truecolor_ansi(&self, idx: usize) -> &'static [u8] {
        ANSI_TRUECOLOR_CACHE[idx]
    }

    /// Get pre-computed 256-color code for a table index
    #[inline(always)]
    #[must_use]
    pub fn get_256_code(&self, idx: usize) -> u8 {
        RAINBOW_256_CODES[idx]
    }

    /// Helper method to compute table index from position
    #[inline(always)]
    fn index_from_position(&self, position: f64) -> usize {
        let k = position.mul_add(self.scale, 0.0) as u64;
        #[allow(clippy::cast_possible_truncation)]
        let idx = (k as usize) & MASK;
        idx
    }

    /// Precompute fixed-point phase & increment for a stream.
    /// Returns (`initial_phase`, `phase_increment`) for integer-only per-glyph updates.
    ///
    /// Use this to eliminate all floating-point math in hot loops.
    #[must_use]
    pub fn fixedpoint_phase(&self, start_pos: f64, pos_increment: f64) -> (u64, u64) {
        const FP_SHIFT: u32 = 32;
        let s = self.scale * ((1u64 << FP_SHIFT) as f64);

        let phase0 = (start_pos * s) as u64;

        let phase_inc = (pos_increment * s) as u64;

        (phase0, phase_inc)
    }

    /// Get color from a fixed-point phase value.
    /// Returns (`Color`, `table_index`) for use with ANSI sequence caching.
    #[inline(always)]
    #[must_use]
    pub fn color_from_phase(&self, phase: u64) -> (Color, usize) {
        let idx = ((phase >> 32) as usize) & MASK;
        (RAINBOW_TABLE[idx], idx)
    }

    /// Get only the table index for a fixed-point phase value.
    ///
    /// This is useful for hot paths that only need the cached ANSI
    /// sequences and can skip fetching the full `Color` struct.
    #[inline(always)]
    #[must_use]
    pub fn color_index_from_phase(&self, phase: u64) -> usize {
        ((phase >> 32) as usize) & MASK
    }

    /// Calculate how many glyphs until the color index changes.
    /// Useful for batching identical color runs.
    #[inline(always)]
    #[must_use]
    pub fn run_len_until_next_index(&self, phase: u64, phase_inc: u64) -> usize {
        if phase_inc == 0 {
            return usize::MAX;
        }

        let hi = phase >> 32;
        let hi_next_boundary = (hi + 1) << 32;
        let delta = hi_next_boundary.wrapping_sub(phase);

        // Ceiling division by phase_inc
        (delta.saturating_add(phase_inc - 1) / phase_inc) as usize
    }
}
