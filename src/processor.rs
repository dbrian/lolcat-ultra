use anyhow::{Context, Result};
use arrayvec::ArrayVec;
use std::io::{self, BufRead, BufWriter, Write};

use crate::ansi::process_ansi_escape_bytes;
use crate::color::{ColorMode, detect_color_support};
use crate::config::Config;
use crate::rainbow::RainbowLookup;

// Include the pre-computed 256-color ANSI cache from build time
// (The rainbow tables are already included in rainbow.rs)
#[allow(dead_code)]
mod generated {
    use crate::color::Color;
    include!(concat!(env!("OUT_DIR"), "/rainbow_tables.rs"));
}

/// Get cached ANSI sequence for a 256-color code
#[inline]
fn get_ansi_256(code: u8) -> &'static [u8] {
    generated::ANSI_256_CACHE[code as usize]
}

/// Buffer capacity for line processing
const BUF_CAP: usize = 8192;

/// Helper to write ANSI TrueColor sequence to buffer
#[inline(always)]
fn write_ansi_truecolor(buf: &mut ArrayVec<u8, BUF_CAP>, color_idx: usize, lookup: &RainbowLookup) {
    buf.try_extend_from_slice(lookup.get_truecolor_ansi(color_idx))
        .unwrap();
}

/// Helper to write ANSI 256-color sequence to buffer
#[inline(always)]
fn write_ansi_256color(buf: &mut ArrayVec<u8, BUF_CAP>, color_idx: usize, lookup: &RainbowLookup) {
    let code = lookup.get_256_code(color_idx);
    buf.try_extend_from_slice(get_ansi_256(code)).unwrap();
}

/// Flush buffer if getting close to capacity
#[inline]
fn maybe_flush<W: Write>(writer: &mut W, buf: &mut ArrayVec<u8, BUF_CAP>) -> io::Result<()> {
    // Leave headroom for ANSI sequences + UTF-8 chars
    if buf.remaining_capacity() < 64 {
        writer.write_all(buf)?;
        buf.clear();
    }
    Ok(())
}

/// Process a line with optimizations:
/// - Pre-cached ANSI sequences (no itoa calls in hot loop)
/// - Stack-allocated buffer (better cache locality)
/// - Single final write (includes newline)
/// - Track last color to avoid redundant ANSI sequences
/// - Single color lookup per character
fn process_line_streaming<W: Write>(
    line: &[u8],
    start_pos: f64,
    config: &Config,
    color_mode: ColorMode,
    lookup: &RainbowLookup,
    writer: &mut W,
) -> Result<()> {
    debug_assert!(start_pos.is_finite(), "Start position must be finite");

    // Dispatch to monomorphic implementation based on color mode
    match color_mode {
        ColorMode::NoColor => {
            // Fast path: no color processing needed
            writer
                .write_all(line)
                .context("Failed to write line without color")?;
            writer
                .write_all(b"\n")
                .context("Failed to write newline without color")?;
            Ok(())
        }
        ColorMode::TrueColor => process_line_with_color(
            line,
            start_pos,
            config,
            lookup,
            writer,
            write_ansi_truecolor,
        ),
        ColorMode::Color256 => {
            process_line_with_color(line, start_pos, config, lookup, writer, write_ansi_256color)
        }
    }
}

/// Monomorphic color processing implementation
/// This function is generic over the ANSI writer to enable complete inlining.
/// Uses byte-level iteration to avoid UTF-8 decoding overhead — only \x1b and \t
/// need detection (both single-byte ASCII). Multi-byte codepoints are copied as
/// raw bytes; the phase counter advances only on codepoint-start bytes.
#[inline]
fn process_line_with_color<W: Write, F>(
    line: &[u8],
    start_pos: f64,
    config: &Config,
    lookup: &RainbowLookup,
    writer: &mut W,
    write_ansi: F,
) -> Result<()>
where
    F: Fn(&mut ArrayVec<u8, BUF_CAP>, usize, &RainbowLookup),
{
    // Stack-allocated buffer - 8KB for better cache locality
    let mut buf = ArrayVec::<u8, BUF_CAP>::new();

    // Fixed-point phase accumulator - eliminates all float ops in hot path
    let pos_increment = 1.0 / config.spread;
    let (mut phase, phase_inc) = lookup.fixedpoint_phase(start_pos, pos_increment);

    // Track last color index to avoid redundant ANSI sequences
    let mut last_color_idx: Option<usize> = None;

    let bytes = line;
    let len = bytes.len();
    let mut i = 0;

    // Optimization: if phase_inc is small, we can process chunks of characters
    // that share the same color index without recalculating it.
    if phase_inc > 0 && phase_inc < (1 << 28) {
        while i < len {
            let b = bytes[i];

            if b == 0x1b {
                // Flush accumulated buffer before ANSI escape
                if !buf.is_empty() {
                    writer.write_all(&buf)?;
                    buf.clear();
                }
                i = process_ansi_escape_bytes(writer, bytes, i)?;
                last_color_idx = None;
                continue;
            }

            if b == b'\t' {
                i += 1;
                for _ in 0..8 {
                    let color_idx = lookup.color_index_from_phase(phase);
                    if last_color_idx != Some(color_idx) {
                        write_ansi(&mut buf, color_idx, lookup);
                        last_color_idx = Some(color_idx);
                    }
                    buf.push(b' ');
                    phase = phase.wrapping_add(phase_inc);
                    maybe_flush(writer, &mut buf)?;
                }
                continue;
            }

            // Normal character batching
            let color_idx = lookup.color_index_from_phase(phase);
            if last_color_idx != Some(color_idx) {
                write_ansi(&mut buf, color_idx, lookup);
                last_color_idx = Some(color_idx);
            }

            let max_run = lookup.run_len_until_next_index(phase, phase_inc);
            let mut processed = 0;

            // Inner loop: consume up to max_run codepoints worth of bytes.
            // We must never break in the middle of a multi-byte UTF-8 sequence,
            // as that would allow an ANSI color code to be inserted between
            // the start byte and continuation bytes, corrupting the character.
            while i < len && buf.remaining_capacity() >= 4 {
                let b2 = bytes[i];
                if b2 == 0x1b || b2 == b'\t' {
                    break;
                }
                // Check codepoint-start: if we've already hit max_run,
                // stop before starting a new codepoint
                if b2 < 0x80 || b2 >= 0xC0 {
                    if processed >= max_run {
                        break;
                    }
                    processed += 1;
                }
                buf.push(b2);
                i += 1;
            }

            if processed > 0 {
                phase = phase.wrapping_add(phase_inc.wrapping_mul(processed as u64));
                maybe_flush(writer, &mut buf)?;
            }
        }
    } else {
        while i < len {
            let b = bytes[i];

            if b == 0x1b {
                // Flush accumulated buffer before ANSI escape
                if !buf.is_empty() {
                    writer.write_all(&buf)?;
                    buf.clear();
                }
                i = process_ansi_escape_bytes(writer, bytes, i)?;
                last_color_idx = None;
                continue;
            }

            if b == b'\t' {
                i += 1;
                for _ in 0..8 {
                    let color_idx = lookup.color_index_from_phase(phase);
                    if last_color_idx != Some(color_idx) {
                        write_ansi(&mut buf, color_idx, lookup);
                        last_color_idx = Some(color_idx);
                    }
                    buf.push(b' ');
                    phase = phase.wrapping_add(phase_inc);
                    maybe_flush(writer, &mut buf)?;
                }
                continue;
            }

            // Only emit color on codepoint-start bytes to avoid splitting
            // multi-byte UTF-8 sequences with ANSI escapes
            if b < 0x80 || b >= 0xC0 {
                let color_idx = lookup.color_index_from_phase(phase);
                if last_color_idx != Some(color_idx) {
                    write_ansi(&mut buf, color_idx, lookup);
                    last_color_idx = Some(color_idx);
                }
            }

            // Copy byte to buffer
            buf.push(b);
            i += 1;

            // Advance phase only on codepoint-start bytes
            if b < 0x80 || b >= 0xC0 {
                phase = phase.wrapping_add(phase_inc);
            }

            maybe_flush(writer, &mut buf)?;
        }
    }

    // Append newline and write in one syscall
    buf.push(b'\n');
    writer
        .write_all(&buf)
        .context("Failed to write final buffered line")?;

    Ok(())
}

/// Optimized batch processing for better performance with large inputs
struct BatchProcessor<W: Write> {
    writer: BufWriter<W>,
    lookup: RainbowLookup,
}

impl<W: Write> BatchProcessor<W> {
    fn new(writer: W, config: &Config) -> Self {
        // Use a larger buffer size for better performance with large files
        const BUFFER_SIZE: usize = 64 * 1024; // 64KB buffer
        Self {
            writer: BufWriter::with_capacity(BUFFER_SIZE, writer),
            lookup: RainbowLookup::new(config.frequency),
        }
    }

    fn process_line(
        &mut self,
        line: &[u8],
        start_pos: f64,
        config: &Config,
        color_mode: ColorMode,
    ) -> Result<()> {
        process_line_streaming(
            line,
            start_pos,
            config,
            color_mode,
            &self.lookup,
            &mut self.writer,
        )?;
        Ok(())
    }

    fn finish(mut self) -> Result<()> {
        // Comprehensive terminal reset sequence
        write!(self.writer, "\x1b[0m\x1b[39m\x1b[49m").context("Failed to write terminal reset")?;
        self.writer.flush().context("Failed to flush final batch")
    }
}

/// Process input with a specific color mode (for testing/benchmarking)
///
/// # Errors
///
/// Returns an error if:
/// - Reading from the input reader fails
/// - Writing to the output writer fails
/// - Maximum line limit is exceeded
pub fn process_input_with_color_mode<R: BufRead, W: Write>(
    mut reader: R,
    writer: W,
    config: &Config,
    color_mode: ColorMode,
) -> Result<()> {
    // Maximum number of lines to process to ensure statically provable upper bound
    // This prevents infinite loops when reading from stdin or very large files
    const MAX_LINES: usize = 1_000_000_000;

    // Fast path: when no color, just copy input to output like cat
    if color_mode == ColorMode::NoColor {
        let mut writer = writer;
        loop {
            let n = reader.fill_buf().context("Failed to read input")?;
            if n.is_empty() {
                break;
            }
            writer.write_all(n).context("Failed to write output")?;
            let n = n.len();
            reader.consume(n);
        }
        return Ok(());
    }

    // Color processing path
    let mut processor = BatchProcessor::new(writer, config);
    let mut line_buf: Vec<u8> = Vec::with_capacity(1024);
    let mut lines_read = 0;

    loop {
        if lines_read >= MAX_LINES {
            // Safety limit reached - prevent unbounded processing
            break;
        }

        line_buf.clear();
        let n = reader
            .read_until(b'\n', &mut line_buf)
            .context("Failed to read line")?;
        if n == 0 {
            break;
        }

        // Trim trailing newline to match lines() behavior
        let mut line_len = line_buf.len();
        if line_buf.last() == Some(&b'\n') {
            line_len -= 1;
            if line_len > 0 && line_buf[line_len - 1] == b'\r' {
                line_len -= 1;
            }
        }
        let line = &line_buf[..line_len];

        // Calculate start position for this line
        let start_pos = (lines_read as f64) * config.spread + config.random_offset;
        debug_assert!(
            start_pos.is_finite(),
            "Calculated start position must be finite"
        );

        processor.process_line(line, start_pos, config, color_mode)?;

        lines_read += 1;
    }

    processor.finish()
}

/// Process input from a reader, applying rainbow colors to each line, writing to a custom writer
///
/// This is primarily for benchmarking and testing purposes.
///
/// # Errors
///
/// Returns an error if:
/// - Reading from the input reader fails
/// - Writing to the output writer fails
/// - Maximum line limit is exceeded
pub fn process_input_to_writer<R: BufRead, W: Write>(
    reader: R,
    writer: W,
    config: &Config,
) -> Result<()> {
    let color_mode = detect_color_support(config.force_color);
    process_input_with_color_mode(reader, writer, config, color_mode)
}

/// Process input from a reader, applying rainbow colors to each line
///
/// # Errors
///
/// Returns an error if:
/// - Reading from the input reader fails
/// - Writing to stdout fails
/// - Maximum line limit is exceeded
pub fn process_input<R: BufRead>(reader: R, config: &Config) -> Result<()> {
    let stdout = io::stdout().lock();
    process_input_to_writer(reader, stdout, config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Cursor};

    /// Strip all ANSI escape sequences from output bytes, returning plain text.
    fn strip_ansi(input: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(input.len());
        let mut i = 0;
        while i < input.len() {
            if input[i] == 0x1b {
                i += 1;
                // Skip CSI sequences: ESC [ ... <letter>
                if i < input.len() && input[i] == b'[' {
                    i += 1;
                    while i < input.len() && !input[i].is_ascii_alphabetic() {
                        i += 1;
                    }
                    if i < input.len() {
                        i += 1; // skip terminating letter
                    }
                }
            } else {
                out.push(input[i]);
                i += 1;
            }
        }
        out
    }

    /// Process input with color and return the plain text (ANSI stripped).
    fn process_and_strip(input: &str, color_mode: ColorMode) -> String {
        let config = Config::try_new(0.04, 4.0, true).unwrap();
        let reader = BufReader::new(Cursor::new(input.as_bytes()));
        let mut output = Vec::new();
        process_input_with_color_mode(reader, &mut output, &config, color_mode).unwrap();
        let stripped = strip_ansi(&output);
        String::from_utf8(stripped).expect("output must be valid UTF-8")
    }

    /// The processor adds a newline after each line. For input ending with
    /// \n, the last split produces an empty segment that doesn't get a line.
    fn expected_output(input: &str) -> String {
        if input.ends_with('\n') {
            // Input already has trailing newline; processor treats it as
            // a line followed by EOF, so output matches input exactly.
            input.to_string()
        } else {
            // No trailing newline; processor adds one.
            format!("{input}\n")
        }
    }

    /// Tab expansion: each \t becomes 8 spaces
    fn expand_tabs(input: &str) -> String {
        input.replace('\t', "        ")
    }

    #[test]
    fn ascii_text_preserved_truecolor() {
        let input = "Hello, world!\nThe quick brown fox jumps over the lazy dog.";
        let result = process_and_strip(input, ColorMode::TrueColor);
        assert_eq!(result, expected_output(input));
    }

    #[test]
    fn ascii_text_preserved_256color() {
        let input = "Hello, world!\nThe quick brown fox jumps over the lazy dog.";
        let result = process_and_strip(input, ColorMode::Color256);
        assert_eq!(result, expected_output(input));
    }

    #[test]
    fn curly_quotes_preserved() {
        let input = "I\u{2019}ve been turning over in my mind";
        let result = process_and_strip(input, ColorMode::TrueColor);
        assert_eq!(result, expected_output(input));
    }

    #[test]
    fn unicode_multibyte_preserved() {
        let input = "Hello 世界 🌈 Привет مرحبا こんにちは café naïve résumé";
        let result = process_and_strip(input, ColorMode::TrueColor);
        assert_eq!(result, expected_output(input));
    }

    #[test]
    fn curly_quotes_256color() {
        let input = "\u{201c}Hello,\u{201d} he said. \u{2018}It\u{2019}s fine.\u{2019}";
        let result = process_and_strip(input, ColorMode::Color256);
        assert_eq!(result, expected_output(input));
    }

    #[test]
    fn emoji_preserved() {
        let input = "Emojis: 😀 🎉 ✨ 🚀 💻 🔥 👨‍👩‍👧‍👦";
        let result = process_and_strip(input, ColorMode::TrueColor);
        assert_eq!(result, expected_output(input));
    }

    #[test]
    fn tabs_expanded() {
        let input = "col1\tcol2\tcol3";
        let result = process_and_strip(input, ColorMode::TrueColor);
        assert_eq!(result, expected_output(&expand_tabs(input)));
    }

    #[test]
    fn empty_and_blank_lines() {
        let input = "\n\n  \n";
        let result = process_and_strip(input, ColorMode::TrueColor);
        assert_eq!(result, expected_output(input));
    }

    #[test]
    fn all_printable_ascii() {
        let input: String = (0x20u8..=0x7E).map(|b| b as char).collect();
        let result = process_and_strip(&input, ColorMode::TrueColor);
        assert_eq!(result, expected_output(&input));
    }

    #[test]
    fn mixed_ascii_and_multibyte_long_line() {
        // Long line that forces buffer flushes with mixed content
        let segment = "abc\u{00e9}def\u{2019}ghi\u{4e16}jkl\u{1F308}mno ";
        let input: String = segment.repeat(200);
        let result = process_and_strip(&input, ColorMode::TrueColor);
        assert_eq!(result, expected_output(&input));
    }

    #[test]
    fn slow_color_change_preserves_text() {
        // Low frequency, high spread → batching path
        let config = Config::try_new(0.001, 10.0, true).unwrap();
        let input = "I\u{2019}ve got \u{201c}curly quotes\u{201d} and caf\u{00e9}\n";
        let reader = BufReader::new(Cursor::new(input.as_bytes()));
        let mut output = Vec::new();
        process_input_with_color_mode(reader, &mut output, &config, ColorMode::TrueColor).unwrap();
        let stripped = strip_ansi(&output);
        let result = String::from_utf8(stripped).expect("output must be valid UTF-8");
        assert_eq!(result, expected_output(input));
    }
}
