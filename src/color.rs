#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColorMode {
    TrueColor,
    Color256,
    NoColor,
}

pub fn detect_color_support(force_color: bool) -> ColorMode {
    use std::env;

    // Normalize once
    let no_color = env::var("NO_COLOR").ok();
    let force_color_env = env::var("FORCE_COLOR").ok();
    let term = env::var("TERM").ok();
    let term_lower = term.as_deref().map(str::to_ascii_lowercase);
    let colorterm = env::var("COLORTERM").ok();
    let colorterm_l = colorterm.as_deref().map(str::to_ascii_lowercase);
    let term_program = env::var("TERM_PROGRAM").ok();
    let term_program_l = term_program.as_deref().map(str::to_ascii_lowercase);

    let wt_session = env::var("WT_SESSION").is_ok();
    let vscode_inj = env::var("VSCODE_INJECTION").is_ok();
    let ci = env::var("CI").is_ok() || env::var("GITHUB_ACTIONS").is_ok();

    // 1) NO_COLOR wins, always
    if no_color.is_some() {
        return ColorMode::NoColor;
    }

    // 2) FORCE_COLOR environment variable (align with widespread conventions)
    if let Some(ref v) = force_color_env {
        // FORCE_COLOR=0 → disable; FORCE_COLOR empty/1/2/3 → enable various levels
        if v == "0" {
            return ColorMode::NoColor;
        }
        // Empty or unparsable → treat like "1" (basic) or "2" (256). We'll choose 256 as a practical default.
        let level = v.parse::<u8>().unwrap_or(2);
        return match level {
            3 => ColorMode::TrueColor,
            _ => ColorMode::Color256,
        };
    }

    // 3) Command-line --force flag
    if force_color {
        return ColorMode::TrueColor;
    }

    // 4) If stdout is not a tty and we haven't been forced, disable color
    if !std::io::IsTerminal::is_terminal(&std::io::stdout()) {
        return ColorMode::NoColor;
    }

    // 5) TERM=dumb/unknown disables unless forced
    if let Some(ref t) = term_lower {
        if t == "dumb" || t == "unknown" {
            return ColorMode::NoColor;
        }
    }

    // 6) Strong truecolor signals
    let has_truecolor_signal = colorterm_l.as_deref().is_some_and(|c| (c.contains("truecolor") || c.contains("24bit"))) ||
        term_program_l
            .as_deref()
            .is_some_and(
                |p|
                    p.contains("iterm") ||
                    p.contains("wezterm") ||
                    p.contains("warp") ||
                    p.contains("alacritty") ||
                    p.contains("ghostty") ||
                    p.contains("apple_terminal")
            ) ||
        wt_session ||
        vscode_inj ||
        // Some TERM values are a dead giveaway
        term_lower
            .as_deref()
            .is_some_and(
                |t|
                    t.contains("xterm-kitty") ||
                    t.contains("alacritty") ||
                    t.contains("wezterm") ||
                    t.contains("ghostty") ||
                    t.contains("konsole") ||
                    t.contains("gnome") ||
                    t.contains("vte") ||
                    t.contains("foot") ||
                    t.contains("iterm")
            );

    if has_truecolor_signal {
        return ColorMode::TrueColor;
    }

    // 7) 256-color signals from TERM
    if let Some(ref t) = term_lower {
        if t.contains("256color") {
            return ColorMode::Color256;
        }
        // tmux/screen often support >16; promote to 256 unless we have a reason not to.
        if t.contains("tmux") || t.contains("screen") {
            return ColorMode::Color256;
        }
    }

    // 8) Basic color signals from TERM
    if let Some(ref t) = term_lower {
        if t.contains("xterm") || t.contains("ansi") || t.contains("vt100") || t.contains("color") {
            // If it wasn't explicitly 256color, use 256 color support.
            return ColorMode::Color256;
        }
    }

    // 9) CI environments: enable at least 256 color
    if ci {
        return ColorMode::Color256;
    }

    // 10) Default to 256-color for tty (we know we're a tty at this point from check #4)
    ColorMode::Color256
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color(pub u8, pub u8, pub u8);

const fn build_scale5() -> [u8; 256] {
    let mut table = [0u8; 256];
    let mut i = 0usize;
    while i < 256 {
        table[i] = (((i as u16) * 5) >> 8) as u8;
        i += 1;
    }
    table
}

const fn build_gray_codes() -> [u8; 256] {
    let mut table = [0u8; 256];
    let mut i = 0usize;
    while i < 256 {
        let value = i as u8;
        table[i] = if value < 8 {
            16
        } else if value > 248 {
            231
        } else {
            232 + (((((value as u16) - 8) * 25) >> 8) as u8)
        };
        i += 1;
    }
    table
}

const SCALE5: [u8; 256] = build_scale5();
const GRAY_CODES: [u8; 256] = build_gray_codes();

#[inline]
#[must_use]
pub const fn rgb_to_256(r: u8, g: u8, b: u8) -> u8 {
    if r == g && g == b {
        GRAY_CODES[r as usize]
    } else {
        let r6 = SCALE5[r as usize] as u16;
        let g6 = SCALE5[g as usize] as u16;
        let b6 = SCALE5[b as usize] as u16;
        (16 + 36 * r6 + 6 * g6 + b6) as u8
    }
}
