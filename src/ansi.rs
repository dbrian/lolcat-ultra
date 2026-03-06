use anyhow::{Context, Result};
use arrayvec::ArrayVec;
use std::io::Write;

pub(crate) const MAX_ANSI_SEQUENCE_LENGTH: usize = 200;

/// Byte-level ANSI escape processor. Reads an ANSI escape sequence starting
/// at `bytes[pos]` (which should be 0x1B) and writes it directly to `writer`.
/// Returns the new position after the escape sequence.
#[inline]
pub(crate) fn process_ansi_escape_bytes<W: Write>(
    writer: &mut W,
    bytes: &[u8],
    pos: usize,
) -> Result<usize> {
    let mut buf = ArrayVec::<u8, { MAX_ANSI_SEQUENCE_LENGTH + 4 }>::new();

    // Push the ESC byte
    buf.push(bytes[pos]);
    let mut i = pos + 1;
    let mut ansi_char_count = 0;
    let end = bytes.len();

    while i < end && ansi_char_count < MAX_ANSI_SEQUENCE_LENGTH {
        let b = bytes[i];
        buf.push(b);
        i += 1;
        ansi_char_count += 1;

        // ANSI sequences end on ASCII alphabetic characters (A–Z, a–z)
        if b.is_ascii_alphabetic() {
            break;
        }
    }

    writer
        .write_all(&buf)
        .context("Failed to write ANSI escape sequence")?;

    Ok(i)
}
