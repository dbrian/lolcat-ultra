use anyhow::{Context, Result};
use std::io::{self, BufRead, Write};

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

/// Capacity of the persistent output buffer. Lines accumulate here and are
/// written to the underlying writer directly once the remaining space drops
/// below `LINE_MARGIN`, so the common case does exactly one copy: tables
/// into the buffer, buffer to the fd.
const OUT_CAP: usize = 256 * 1024;

/// Guaranteed headroom at the start of each line (the caller flushes below
/// this). Lines whose worst-case colored size fits in this margin take the
/// checkless fast paths.
const LINE_MARGIN: usize = 8192;

/// Flush margin for the unbounded-line paths: worst case appended between
/// flush checks is a tab (8 colored spaces = 8 * 21 bytes) plus slack.
const FLUSH_MARGIN: usize = 224;

/// Write ANSI TrueColor sequence into `buf` at `j`.
/// Returns the offset where the following character byte must be stored.
/// Copies the full 20-byte fixed-width table entry (19 content bytes + 1
/// padding byte) so the compiler emits a fixed-size copy; the padding slot
/// at the returned offset is overwritten by the caller's character byte.
#[inline(always)]
fn write_ansi_truecolor(
    buf: &mut [u8; OUT_CAP],
    j: usize,
    color_idx: usize,
    lookup: &RainbowLookup,
) -> usize {
    buf[j..j + 20].copy_from_slice(lookup.get_truecolor_ansi_fixed(color_idx));
    j + 19
}

/// Write ANSI 256-color sequence into `buf` at `j`.
/// Returns the offset where the following character byte must be stored.
#[inline(always)]
fn write_ansi_256color(
    buf: &mut [u8; OUT_CAP],
    j: usize,
    color_idx: usize,
    lookup: &RainbowLookup,
) -> usize {
    let seq = get_ansi_256(lookup.get_256_code(color_idx));
    buf[j..j + seq.len()].copy_from_slice(seq);
    j + seq.len()
}

/// Process a line with optimizations:
/// - Pre-cached ANSI sequences (no itoa calls in hot loop)
/// - Reused flat output buffer with index-based writes (no per-push bookkeeping)
/// - Single final write (includes newline)
/// - Track last color to avoid redundant ANSI sequences
/// - Single color lookup per character
/// Appends the colored line to `out` starting at cursor `j` and returns the
/// new cursor. The caller guarantees at least `LINE_MARGIN` bytes of headroom
/// at entry; oversized lines flush `out` to `writer` mid-line as needed.
fn process_line_streaming<W: Write>(
    line: &[u8],
    phase0: u64,
    phase_inc: u64,
    color_mode: ColorMode,
    lookup: &RainbowLookup,
    out: &mut [u8; OUT_CAP],
    j: usize,
    writer: &mut W,
) -> Result<usize> {
    // Dispatch to monomorphic implementation based on color mode
    match color_mode {
        ColorMode::NoColor => {
            // Fast path: no color processing needed. Bypass the buffer since
            // the line may be arbitrarily long.
            if j > 0 {
                writer
                    .write_all(&out[..j])
                    .context("Failed to flush before uncolored line")?;
            }
            writer
                .write_all(line)
                .context("Failed to write line without color")?;
            writer
                .write_all(b"\n")
                .context("Failed to write newline without color")?;
            Ok(0)
        }
        ColorMode::TrueColor => process_line_with_color::<_, _, true>(
            line,
            phase0,
            phase_inc,
            lookup,
            out,
            j,
            writer,
            write_ansi_truecolor,
        ),
        ColorMode::Color256 => process_line_with_color::<_, _, false>(
            line,
            phase0,
            phase_inc,
            lookup,
            out,
            j,
            writer,
            write_ansi_256color,
        ),
    }
}

/// Monomorphic color processing implementation
/// This function is generic over the ANSI writer to enable complete inlining.
/// Uses byte-level iteration to avoid UTF-8 decoding overhead — only \x1b and \t
/// need detection (both single-byte ASCII). Multi-byte codepoints are copied as
/// raw bytes; the phase counter advances only on codepoint-start bytes.
#[inline]
#[allow(clippy::too_many_arguments)]
fn process_line_with_color<W: Write, F, const FIXED_ANSI: bool>(
    line: &[u8],
    phase0: u64,
    phase_inc: u64,
    lookup: &RainbowLookup,
    out: &mut [u8; OUT_CAP],
    j: usize,
    writer: &mut W,
    write_ansi: F,
) -> Result<usize>
where
    F: Fn(&mut [u8; OUT_CAP], usize, usize, &RainbowLookup) -> usize,
{
    // Fixed-point phase accumulator - no float ops anywhere in the hot path
    let mut phase = phase0;

    // Track last color index to avoid redundant ANSI sequences
    let mut last_color_idx: Option<usize> = None;

    let bytes = line;
    let len = bytes.len();
    let mut i = 0;
    // Write cursor into `out`. ANSI writers return the offset for the
    // following character byte (overwriting the truecolor padding slot).
    let mut j = j;

    // Optimization: if phase_inc is small, we can process chunks of characters
    // that share the same color index without recalculating it.
    if phase_inc > 0 && phase_inc < (1 << 28) {
        while i < len {
            let b = bytes[i];

            if b == 0x1b {
                // Flush accumulated buffer before ANSI escape
                if j > 0 {
                    writer.write_all(&out[..j])?;
                    j = 0;
                }
                i = process_ansi_escape_bytes(writer, bytes, i)?;
                last_color_idx = None;
                continue;
            }

            if b == b'\t' {
                i += 1;
                if OUT_CAP - j < FLUSH_MARGIN {
                    writer.write_all(&out[..j])?;
                    j = 0;
                }
                for _ in 0..8 {
                    let color_idx = lookup.color_index_from_phase(phase);
                    if last_color_idx != Some(color_idx) {
                        j = write_ansi(out, j, color_idx, lookup);
                        last_color_idx = Some(color_idx);
                    }
                    out[j] = b' ';
                    j += 1;
                    phase = phase.wrapping_add(phase_inc);
                }
                continue;
            }

            // Normal character batching
            if OUT_CAP - j < FLUSH_MARGIN {
                writer.write_all(&out[..j])?;
                j = 0;
            }
            let color_idx = lookup.color_index_from_phase(phase);
            if last_color_idx != Some(color_idx) {
                j = write_ansi(out, j, color_idx, lookup);
                last_color_idx = Some(color_idx);
            }

            let max_run = lookup.run_len_until_next_index(phase, phase_inc);
            let mut processed = 0;

            // Inner loop: consume up to max_run codepoints worth of bytes.
            // We must never break in the middle of a multi-byte UTF-8 sequence,
            // as that would allow an ANSI color code to be inserted between
            // the start byte and continuation bytes, corrupting the character.
            // The first iteration always writes (bytes[i] is a normal char and
            // max_run >= 1), so the truecolor padding slot is always overwritten.
            while i < len && OUT_CAP - j > 4 {
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
                out[j] = b2;
                j += 1;
                i += 1;
            }

            if processed > 0 {
                phase = phase.wrapping_add(phase_inc.wrapping_mul(processed as u64));
            }
        }
    } else {
        // Fast path: short pure-ASCII lines with no ESC or tab (the common case).
        // Each char needs at most 20 bytes (19-byte ANSI + 1-byte char, sharing
        // the padding slot). If the entire line fits in the buffer AND contains
        // no special bytes, skip per-char ESC/tab/capacity/UTF-8 checks entirely.
        let fits_in_buf = len <= (LINE_MARGIN - 24) / 20;
        // Single branchless pass classifying the line (vectorizes)
        let mut has_special = false;
        let mut has_non_ascii = false;
        for &b in bytes {
            has_special |= (b == 0x1b) | (b == b'\t');
            has_non_ascii |= b >= 0x80;
        }
        let no_special = fits_in_buf && !has_special;
        let all_ascii = no_special && !has_non_ascii;

        if all_ascii {
            // Tightest inner loop: pure ASCII, no special bytes, buffer won't fill.
            // No ESC/tab/capacity/UTF-8 checks needed.
            if phase_inc >= (1 << 32) {
                // The color index advances at least once per character, so the
                // ANSI sequence always changes: emit unconditionally with no
                // last-color tracking or branches.
                if FIXED_ANSI {
                    // TrueColor emits exactly 20 bytes per character (19-byte
                    // sequence + the character in the padding slot). Slice the
                    // destination once and iterate in exact 20-byte chunks:
                    // no per-character bounds checks at all.
                    let dst = &mut out[j..j + len * 20];
                    for (chunk, &b) in dst.chunks_exact_mut(20).zip(bytes) {
                        let color_idx = lookup.color_index_from_phase(phase);
                        chunk.copy_from_slice(lookup.get_truecolor_ansi_fixed(color_idx));
                        chunk[19] = b;
                        phase = phase.wrapping_add(phase_inc);
                    }
                    j += len * 20;
                } else {
                    while i < len {
                        let color_idx = lookup.color_index_from_phase(phase);
                        j = write_ansi(out, j, color_idx, lookup);
                        out[j] = bytes[i];
                        j += 1;
                        phase = phase.wrapping_add(phase_inc);
                        i += 1;
                    }
                }
            } else {
                while i < len {
                    let color_idx = lookup.color_index_from_phase(phase);
                    if last_color_idx != Some(color_idx) {
                        j = write_ansi(out, j, color_idx, lookup);
                        last_color_idx = Some(color_idx);
                    }
                    phase = phase.wrapping_add(phase_inc);
                    out[j] = bytes[i];
                    j += 1;
                    i += 1;
                }
            }
        } else if no_special {
            // ASCII + UTF-8 but no ESC/tab; no capacity check needed.
            if phase_inc >= (1 << 32) {
                // Color index changes on every codepoint: emit unconditionally.
                while i < len {
                    let b = bytes[i];
                    if b < 0x80 || b >= 0xC0 {
                        let color_idx = lookup.color_index_from_phase(phase);
                        j = write_ansi(out, j, color_idx, lookup);
                        phase = phase.wrapping_add(phase_inc);
                    }
                    out[j] = b;
                    j += 1;
                    i += 1;
                }
            } else {
                while i < len {
                    let b = bytes[i];
                    if b < 0x80 || b >= 0xC0 {
                        let color_idx = lookup.color_index_from_phase(phase);
                        if last_color_idx != Some(color_idx) {
                            j = write_ansi(out, j, color_idx, lookup);
                            last_color_idx = Some(color_idx);
                        }
                        phase = phase.wrapping_add(phase_inc);
                    }
                    out[j] = b;
                    j += 1;
                    i += 1;
                }
            }
        } else {
            while i < len {
                let b = bytes[i];

                if b == 0x1b {
                    // Flush accumulated buffer before ANSI escape
                    if j > 0 {
                        writer.write_all(&out[..j])?;
                        j = 0;
                    }
                    i = process_ansi_escape_bytes(writer, bytes, i)?;
                    last_color_idx = None;
                    continue;
                }

                if b == b'\t' {
                    i += 1;
                    if OUT_CAP - j < FLUSH_MARGIN {
                        writer.write_all(&out[..j])?;
                        j = 0;
                    }
                    for _ in 0..8 {
                        let color_idx = lookup.color_index_from_phase(phase);
                        if last_color_idx != Some(color_idx) {
                            j = write_ansi(out, j, color_idx, lookup);
                            last_color_idx = Some(color_idx);
                        }
                        out[j] = b' ';
                        j += 1;
                        phase = phase.wrapping_add(phase_inc);
                    }
                    continue;
                }

                // Codepoint-start bytes: flush if needed, emit color, advance phase.
                // Continuation bytes (0x80–0xBF) just get pushed; headroom is guaranteed
                // by the flush check on each start byte (max 20-byte ANSI + 4-byte codepoint < 32).
                if b < 0x80 || b >= 0xC0 {
                    if OUT_CAP - j < 32 {
                        writer.write_all(&out[..j])?;
                        j = 0;
                    }
                    let color_idx = lookup.color_index_from_phase(phase);
                    if last_color_idx != Some(color_idx) {
                        j = write_ansi(out, j, color_idx, lookup);
                        last_color_idx = Some(color_idx);
                    }
                    phase = phase.wrapping_add(phase_inc);
                }

                out[j] = b;
                j += 1;
                i += 1;
            }
        }
    }

    // Append newline; the accumulated output is flushed by the caller
    out[j] = b'\n';
    j += 1;

    Ok(j)
}

/// Optimized batch processing for better performance with large inputs
struct BatchProcessor<W: Write> {
    writer: W,
    /// Persistent output buffer: lines accumulate here (index-based writes,
    /// no per-push bookkeeping) and are written straight to the underlying
    /// writer when headroom runs low — a single copy from tables to buffer.
    out: Box<[u8; OUT_CAP]>,
    /// Write cursor into `out`, carried across lines
    j: usize,
    /// Fixed-point phase at the start of the current line
    phase: u64,
    /// Fixed-point phase advance per line (spread positions)
    line_phase_inc: u64,
    /// Fixed-point phase advance per character
    phase_inc: u64,
    lookup: RainbowLookup,
}

impl<W: Write> BatchProcessor<W> {
    fn new(writer: W, config: &Config) -> Self {
        let lookup = RainbowLookup::new(config.frequency);
        // All phase math is precomputed once: the line start phase advances
        // incrementally by `line_phase_inc` per line instead of being derived
        // from floats per line. The truncation drift versus exact per-line
        // float math stays far below one table index over billions of lines.
        let (phase, phase_inc) = lookup.fixedpoint_phase(config.random_offset, 1.0 / config.spread);
        let (line_phase_inc, _) = lookup.fixedpoint_phase(config.spread, 1.0);
        Self {
            writer,
            out: Box::new([0u8; OUT_CAP]),
            j: 0,
            phase,
            line_phase_inc,
            phase_inc,
            lookup,
        }
    }

    /// Whether the fused clean-ASCII chunk path applies: TrueColor output and
    /// a color index that advances on every character.
    fn fused_chunk_eligible(&self, color_mode: ColorMode) -> bool {
        color_mode == ColorMode::TrueColor && self.phase_inc >= (1 << 32)
    }

    /// Process every complete line of a pre-verified clean chunk (pure ASCII,
    /// no ESC/tab/CR) with zero per-line classification or dispatch.
    /// Returns (bytes consumed, lines processed), capped at `line_limit`.
    fn process_clean_ascii_chunk(
        &mut self,
        chunk: &[u8],
        line_limit: usize,
    ) -> Result<(usize, usize)> {
        let mut j = self.j;
        let mut phase_line = self.phase;
        let mut offset = 0;
        let mut lines = 0;
        for nl in memchr::memchr_iter(b'\n', chunk) {
            let line = &chunk[offset..nl];
            if OUT_CAP - j < LINE_MARGIN {
                self.writer
                    .write_all(&self.out[..j])
                    .context("Failed to write output batch")?;
                j = 0;
            }
            if line.len() <= (LINE_MARGIN - 24) / 20 {
                let mut phase = phase_line;
                let dst = &mut self.out[j..j + line.len() * 20];
                for (c, &b) in dst.chunks_exact_mut(20).zip(line) {
                    let color_idx = self.lookup.color_index_from_phase(phase);
                    c.copy_from_slice(self.lookup.get_truecolor_ansi_fixed(color_idx));
                    c[19] = b;
                    phase = phase.wrapping_add(self.phase_inc);
                }
                j += line.len() * 20;
                self.out[j] = b'\n';
                j += 1;
            } else {
                // Oversized line: the general path handles mid-line flushing
                j = process_line_streaming(
                    line,
                    phase_line,
                    self.phase_inc,
                    ColorMode::TrueColor,
                    &self.lookup,
                    &mut self.out,
                    j,
                    &mut self.writer,
                )?;
            }
            phase_line = phase_line.wrapping_add(self.line_phase_inc);
            offset = nl + 1;
            lines += 1;
            if lines >= line_limit {
                break;
            }
        }
        self.j = j;
        self.phase = phase_line;
        Ok((offset, lines))
    }

    fn process_line(&mut self, line: &[u8], color_mode: ColorMode) -> Result<()> {
        // Guarantee LINE_MARGIN headroom so short lines run checkless
        if OUT_CAP - self.j < LINE_MARGIN {
            self.writer
                .write_all(&self.out[..self.j])
                .context("Failed to write output batch")?;
            self.j = 0;
        }
        self.j = process_line_streaming(
            line,
            self.phase,
            self.phase_inc,
            color_mode,
            &self.lookup,
            &mut self.out,
            self.j,
            &mut self.writer,
        )?;
        self.phase = self.phase.wrapping_add(self.line_phase_inc);
        Ok(())
    }

    fn finish(mut self) -> Result<()> {
        self.writer
            .write_all(&self.out[..self.j])
            .context("Failed to write final batch")?;
        // Comprehensive terminal reset sequence
        self.writer
            .write_all(b"\x1b[0m\x1b[39m\x1b[49m")
            .context("Failed to write terminal reset")?;
        self.writer.flush().context("Failed to flush final batch")
    }
}

/// Whether the chunk is pure ASCII with no ESC, tab, or CR — eligible for the
/// fused colorize path. Scans blockwise (branchless within a block, so it
/// vectorizes) with early exit between blocks for quick rejection.
fn is_clean_ascii(chunk: &[u8]) -> bool {
    for block in chunk.chunks(1024) {
        let mut dirty = false;
        for &b in block {
            dirty |= (b >= 0x80) | (b == 0x1b) | (b == b'\t') | (b == b'\r');
        }
        if dirty {
            return false;
        }
    }
    true
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
    // line_buf is only used for the rare case where a line spans two buffer fills
    let mut line_buf: Vec<u8> = Vec::with_capacity(1024);
    let mut lines_read = 0;

    loop {
        if lines_read >= MAX_LINES {
            break;
        }

        // Fast path: process every complete line in the reader's buffer
        // (zero copy), finding newlines with a single SIMD scan per chunk
        // and consuming the chunk once.
        let consumed = {
            let available = reader.fill_buf().context("Failed to read input")?;
            if available.is_empty() {
                break;
            }
            if processor.fused_chunk_eligible(color_mode) && is_clean_ascii(available) {
                let (consumed, lines) =
                    processor.process_clean_ascii_chunk(available, MAX_LINES - lines_read)?;
                lines_read += lines;
                consumed
            } else {
                let mut offset = 0;
                for nl in memchr::memchr_iter(b'\n', available) {
                    let before_nl = &available[offset..nl];
                    let line = if before_nl.last() == Some(&b'\r') {
                        &before_nl[..before_nl.len() - 1]
                    } else {
                        before_nl
                    };
                    processor.process_line(line, color_mode)?;
                    lines_read += 1;
                    offset = nl + 1;
                    if lines_read >= MAX_LINES {
                        break;
                    }
                }
                offset
            }
        };

        if consumed > 0 {
            reader.consume(consumed);
        } else {
            // Slow path: line spans a buffer boundary — fall back to read_until
            line_buf.clear();
            let n = reader
                .read_until(b'\n', &mut line_buf)
                .context("Failed to read line")?;
            if n == 0 {
                break;
            }
            let mut line_len = line_buf.len();
            if line_buf.last() == Some(&b'\n') {
                line_len -= 1;
                if line_len > 0 && line_buf[line_len - 1] == b'\r' {
                    line_len -= 1;
                }
            }
            processor.process_line(&line_buf[..line_len], color_mode)?;
            lines_read += 1;
        }
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
