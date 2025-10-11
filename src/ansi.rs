use anyhow::{Context, Result};
use arrayvec::ArrayVec;
use std::io::Write;

pub(crate) const MAX_ANSI_SEQUENCE_LENGTH: usize = 200;

#[inline]
pub(crate) fn process_ansi_escape<W: Write>(
    writer: &mut W,
    chars: &mut std::iter::Peekable<std::str::Chars>,
    initial_char: char,
) -> Result<()> {
    // Preallocate for worst-case UTF-8: 200 chars × 4 bytes + initial char (4 bytes)
    // In practice, ANSI sequences are ASCII (~20 bytes), but this ensures safety
    let mut buf = ArrayVec::<u8, { (MAX_ANSI_SEQUENCE_LENGTH * 4) + 4 }>::new();

    // Encode initial character (usually ESC or '[')
    {
        let mut tmp = [0u8; 4];
        buf.try_extend_from_slice(initial_char.encode_utf8(&mut tmp).as_bytes())
            .unwrap();
    }

    let mut ansi_char_count = 0;
    while let Some(&next) = chars.peek() {
        if ansi_char_count >= MAX_ANSI_SEQUENCE_LENGTH {
            break;
        }

        // Encode next char directly into buffer
        let mut tmp = [0u8; 4];
        buf.try_extend_from_slice(next.encode_utf8(&mut tmp).as_bytes())
            .unwrap();

        chars.next();
        ansi_char_count += 1;

        // ANSI sequences end on ASCII alphabetic characters (A–Z, a–z)
        if next.is_ascii_alphabetic() {
            break;
        }
    }

    // Single system call / buffer write
    writer
        .write_all(&buf)
        .context("Failed to write ANSI escape sequence")
}
