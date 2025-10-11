use anyhow::Result;
use std::io::{self, Write};

/// Ensure terminal is reset on program exit
pub fn setup_terminal_cleanup() {
    // Set up a cleanup function that will run on program exit
    std::panic::set_hook(Box::new(|_| {
        let _ = reset_terminal();
    }));
}

/// Reset terminal to clean state
pub(crate) fn reset_terminal() -> Result<()> {
    let mut stdout = io::stdout();
    write!(stdout, "\x1b[0m\x1b[39m\x1b[49m")?;
    stdout.flush()?;
    Ok(())
}
