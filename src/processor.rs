use anyhow::{Context, Result};
use arrayvec::ArrayVec;
use std::io::{self, BufRead, BufWriter, Write};

use crate::ansi::process_ansi_escape;
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
    line: &str,
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
                .write_all(line.as_bytes())
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
/// This function is generic over the ANSI writer to enable complete inlining
#[inline]
fn process_line_with_color<W: Write, F>(
    line: &str,
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

    let mut chars = line.chars().peekable();

    // Optimization: if phase_inc is small, we can process chunks of characters
    // that share the same color index without recalculating it.
    // Threshold chosen to balance division cost vs batch benefit.
    // 1 << 28 corresponds to approx 16 steps per color index change.
    if phase_inc > 0 && phase_inc < (1 << 28) {
        while chars.peek().is_some() {
            let c_peek = *chars.peek().unwrap();

            if c_peek == '\x1b' {
                let c = chars.next().unwrap();
                // Flush accumulated buffer before ANSI escape
                if !buf.is_empty() {
                    writer.write_all(&buf)?;
                    buf.clear();
                }
                process_ansi_escape(writer, &mut chars, c)?;
                // Reset color tracking after ANSI escape
                last_color_idx = None;
                continue;
            } else if c_peek == '\t' {
                let _ = chars.next().unwrap();
                // Expand tab as 8 spaces
                for _ in 0..8 {
                    // Fast fixed-point color lookup - no float math
                    let color_idx = lookup.color_index_from_phase(phase);

                    // Only output ANSI if color changed
                    if last_color_idx != Some(color_idx) {
                        write_ansi(&mut buf, color_idx, lookup);
                        last_color_idx = Some(color_idx);
                    }

                    buf.push(b' ');
                    phase = phase.wrapping_add(phase_inc);

                    // Smart flush based on remaining capacity
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

            // Inner loop: consume up to max_run normal chars
            // Also check buffer capacity to prevent overflow
            while processed < max_run && buf.remaining_capacity() >= 4 {
                if let Some(&c) = chars.peek() {
                    if c == '\x1b' || c == '\t' {
                        break;
                    }
                    chars.next(); // consume

                    if c.is_ascii() {
                        buf.push(c as u8);
                    } else {
                        let mut utf8 = [0u8; 4];
                        let char_bytes = c.encode_utf8(&mut utf8).as_bytes();
                        buf.try_extend_from_slice(char_bytes).unwrap();
                    }
                    processed += 1;
                } else {
                    break;
                }
            }

            if processed > 0 {
                phase = phase.wrapping_add(phase_inc.wrapping_mul(processed as u64));
                maybe_flush(writer, &mut buf)?;
            }
        }
    } else {
        while let Some(c) = chars.next() {
            match c {
                '\x1b' => {
                    // Flush accumulated buffer before ANSI escape
                    if !buf.is_empty() {
                        writer.write_all(&buf)?;
                        buf.clear();
                    }
                    process_ansi_escape(writer, &mut chars, c)?;
                    // Reset color tracking after ANSI escape
                    last_color_idx = None;
                }
                '\t' => {
                    // Expand tab as 8 spaces
                    for _ in 0..8 {
                        // Fast fixed-point color lookup - no float math
                        let color_idx = lookup.color_index_from_phase(phase);

                        // Only output ANSI if color changed
                        if last_color_idx != Some(color_idx) {
                            write_ansi(&mut buf, color_idx, lookup);
                            last_color_idx = Some(color_idx);
                        }

                        buf.push(b' ');
                        phase = phase.wrapping_add(phase_inc);

                        // Smart flush based on remaining capacity
                        maybe_flush(writer, &mut buf)?;
                    }
                }
                _ => {
                    // Fast fixed-point color lookup - no float math
                    let color_idx = lookup.color_index_from_phase(phase);

                    // Only output ANSI if color changed
                    if last_color_idx != Some(color_idx) {
                        write_ansi(&mut buf, color_idx, lookup);
                        last_color_idx = Some(color_idx);
                    }

                    // Write character - use ASCII fast path when possible
                    if c.is_ascii() {
                        buf.push(c as u8);
                    } else {
                        let mut utf8 = [0u8; 4];
                        let char_bytes = c.encode_utf8(&mut utf8).as_bytes();
                        buf.try_extend_from_slice(char_bytes).unwrap();
                    }

                    phase = phase.wrapping_add(phase_inc);

                    // Smart flush based on remaining capacity
                    maybe_flush(writer, &mut buf)?;
                }
            }
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
        line: &str,
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
    let mut line_buf = String::with_capacity(1024);
    let mut lines_read = 0;

    loop {
        if lines_read >= MAX_LINES {
            // Safety limit reached - prevent unbounded processing
            break;
        }

        line_buf.clear();
        let n = reader
            .read_line(&mut line_buf)
            .context("Failed to read line")?;
        if n == 0 {
            break;
        }

        // Trim trailing newline to match lines() behavior
        let mut line_len = line_buf.len();
        if line_buf.ends_with('\n') {
            line_len -= 1;
            if line_len > 0 && line_buf.as_bytes()[line_len - 1] == b'\r' {
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
