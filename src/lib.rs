mod ansi;
pub mod color;
mod config;
mod processor;
pub mod rainbow;
mod terminal;

// Re-export public API
pub use color::ColorMode;
pub use config::{Config, ConfigError};
pub use processor::{process_input, process_input_to_writer, process_input_with_color_mode};
pub use terminal::setup_terminal_cleanup;
