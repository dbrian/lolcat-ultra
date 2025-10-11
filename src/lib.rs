mod ansi;
pub mod color;
mod config;
mod processor;
pub mod rainbow;
mod terminal;

// Re-export public API
pub use config::{Config, ConfigError};
pub use processor::process_input;
pub use terminal::setup_terminal_cleanup;
