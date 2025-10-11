use std::fmt;

/// Configuration for the rainbow effect
pub struct Config {
    /// Frequency of color changes (higher values mean faster color transitions)
    pub frequency: f64,
    /// Number of characters per rainbow spread
    pub spread: f64,
    /// Random offset for the starting color
    pub(crate) random_offset: f64,
    /// Force color output even when stdout is not a tty
    pub(crate) force_color: bool,
}

#[derive(Debug)]
pub enum ConfigError {
    InvalidFrequency(f64),
    InvalidSpread(f64),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFrequency(freq) => {
                if freq.is_infinite() {
                    write!(f, "invalid frequency: infinite")
                } else if freq.is_nan() {
                    write!(f, "invalid frequency: NaN")
                } else {
                    write!(f, "invalid frequency: {freq}")
                }
            }
            Self::InvalidSpread(spread) => {
                if spread.is_infinite() {
                    write!(f, "invalid spread: infinite")
                } else if spread.is_nan() {
                    write!(f, "invalid spread: NaN")
                } else {
                    write!(f, "invalid spread: {spread}")
                }
            }
        }
    }
}

impl std::error::Error for ConfigError {}

/// Generate a pseudo-random offset based on process ID
///
/// Uses process ID with Knuth's multiplicative hash for fast, deterministic randomness
/// that varies per process invocation without syscall overhead.
/// Implementation follows TAOCP Vol 3, Section 6.4.
fn generate_random_offset() -> f64 {
    let pid = std::process::id(); // u32
    // Knuth's multiplicative hash (TAOCP Vol 3, Section 6.4)
    // Constant 2654435769 ≈ 2^32 / φ (golden ratio)
    let hash = pid.wrapping_mul(2_654_435_769_u32);
    // Range reduction: use all bits (high-quality), normalize to 0.0-1000.0
    f64::from(hash) / f64::from(u32::MAX) * 1000.0
}

impl Config {
    /// Create a new configuration with specified frequency and spread
    ///
    /// A random offset will be automatically generated based on current time
    ///
    /// # Errors
    ///
    /// Returns `ConfigError` if frequency or spread are not finite positive numbers
    pub fn try_new(frequency: f64, spread: f64, force_color: bool) -> Result<Self, ConfigError> {
        if !frequency.is_finite() || frequency <= 0.0 {
            return Err(ConfigError::InvalidFrequency(frequency));
        }
        if !spread.is_finite() || spread <= 0.0 {
            return Err(ConfigError::InvalidSpread(spread));
        }

        Ok(Self {
            frequency,
            spread,
            random_offset: generate_random_offset(),
            force_color,
        })
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            frequency: 0.1,
            spread: 8.0,
            random_offset: generate_random_offset(),
            force_color: false,
        }
    }
}
