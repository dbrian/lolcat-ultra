use std::env;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

const AMPLITUDE: f64 = 127.0;
const OFFSET: f64 = 128.0;
const C: f64 = -0.5; // cos(2π/3)
const S: f64 = 0.866_025_403_784_438_6_f64; // sin(2π/3) = √3/2
const TABLE_SIZE: usize = 2048;

#[derive(Clone, Copy)]
struct Color(u8, u8, u8);

/// Branch-free saturating f64 to u8 converter
fn fast_f64_to_u8_sat(x: f64) -> u8 {
    let y = x.clamp(0.0, 255.0) + 0.5;
    y as u8
}

/// Convert RGB to 256-color palette (duplicated from color.rs for build script)
const fn rgb_to_256(r: u8, g: u8, b: u8) -> u8 {
    if r == g && g == b {
        if r < 8 {
            16
        } else if r > 248 {
            231
        } else {
            232 + (((((r as u16) - 8) * 25) >> 8) as u8)
        }
    } else {
        // Approximate division by 51 with bit shift
        let r6 = ((r as u16) * 5) >> 8;
        let g6 = ((g as u16) * 5) >> 8;
        let b6 = ((b as u16) * 5) >> 8;
        (16 + 36 * r6 + 6 * g6 + b6) as u8
    }
}

/// Build frequency-agnostic rainbow color table using trig recurrence
fn build_table() -> [Color; TABLE_SIZE] {
    let mut arr = [Color(0, 0, 0); TABLE_SIZE];

    let delta = std::f64::consts::TAU / (TABLE_SIZE as f64);
    let (mut sx, mut cx) = (0.0_f64).sin_cos();
    let (sd, cd) = delta.sin_cos();

    for color in &mut arr {
        let r = fast_f64_to_u8_sat(sx.mul_add(AMPLITUDE, OFFSET));
        let g = fast_f64_to_u8_sat((sx * C + cx * S).mul_add(AMPLITUDE, OFFSET));
        let b = fast_f64_to_u8_sat((sx * C - cx * S).mul_add(AMPLITUDE, OFFSET));
        *color = Color(r, g, b);

        let ns = sx.mul_add(cd, cx * sd);
        let nc = cx.mul_add(cd, -sx * sd);
        sx = ns;
        cx = nc;
    }

    arr
}

/// Format a byte sequence as a Rust byte array literal
fn format_byte_array(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 5 + 10);
    s.push_str("&[");
    for (i, &b) in bytes.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        write!(s, "{b}").unwrap();
    }
    s.push(']');
    s
}

fn main() {
    let out_dir = env::var_os("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("rainbow_tables.rs");
    let mut f = BufWriter::new(File::create(&dest_path).unwrap());

    let table = build_table();

    // Write the color table with cache-line alignment for better prefetching
    writeln!(f, "// Auto-generated rainbow color table").unwrap();
    writeln!(f, "#[repr(align(64))]").unwrap();
    writeln!(f, "struct AlignedRainbowTable([Color; {TABLE_SIZE}]);").unwrap();
    writeln!(
        f,
        "static ALIGNED_RAINBOW: AlignedRainbowTable = AlignedRainbowTable(["
    )
    .unwrap();
    for color in &table {
        writeln!(f, "    Color({}, {}, {}),", color.0, color.1, color.2).unwrap();
    }
    writeln!(f, "]);").unwrap();
    writeln!(
        f,
        "pub(crate) const RAINBOW_TABLE: &[Color; {TABLE_SIZE}] = &ALIGNED_RAINBOW.0;"
    )
    .unwrap();
    writeln!(f).unwrap();

    // Write the ANSI truecolor cache
    writeln!(f, "// Auto-generated ANSI truecolor sequences").unwrap();
    writeln!(
        f,
        "pub(crate) static ANSI_TRUECOLOR_CACHE: [&[u8]; {TABLE_SIZE}] = ["
    )
    .unwrap();

    for color in &table {
        // Build ANSI sequence for this color
        let mut seq = Vec::with_capacity(20);
        seq.extend_from_slice(b"\x1b[38;2;");
        seq.extend_from_slice(color.0.to_string().as_bytes());
        seq.push(b';');
        seq.extend_from_slice(color.1.to_string().as_bytes());
        seq.push(b';');
        seq.extend_from_slice(color.2.to_string().as_bytes());
        seq.push(b'm');

        writeln!(f, "    {},", format_byte_array(&seq)).unwrap();
    }
    writeln!(f, "];").unwrap();
    writeln!(f).unwrap();

    // Write the 256-color code cache for rainbow table
    writeln!(f, "// Auto-generated 256-color codes for rainbow table").unwrap();
    writeln!(
        f,
        "pub(crate) const RAINBOW_256_CODES: [u8; {TABLE_SIZE}] = ["
    )
    .unwrap();
    for color in &table {
        let code_256 = rgb_to_256(color.0, color.1, color.2);
        write!(f, "{code_256},").unwrap();
    }
    writeln!(f, "];").unwrap();
    writeln!(f).unwrap();

    // Write the 256-color ANSI cache
    writeln!(f, "// Auto-generated 256-color ANSI sequences").unwrap();
    writeln!(f, "#[allow(dead_code)]").unwrap();
    writeln!(f, "pub(crate) const ANSI_256_CACHE: [&[u8]; 256] = [").unwrap();

    for code in 0..256 {
        let mut seq = Vec::with_capacity(16);
        seq.extend_from_slice(b"\x1b[38;5;");
        seq.extend_from_slice(code.to_string().as_bytes());
        seq.push(b'm');
        writeln!(f, "    {},", format_byte_array(&seq)).unwrap();
    }
    writeln!(f, "];").unwrap();

    drop(f);

    // Write the rgb_to_256 lookup tables so they can be included without runtime work.
    let color_dest_path = Path::new(&out_dir).join("color_tables.rs");
    let mut color_file = BufWriter::new(File::create(color_dest_path).unwrap());

    writeln!(color_file, "// Auto-generated 256-color quantizer tables").unwrap();
    writeln!(color_file, "pub(crate) const SCALE5: [u8; 256] = [").unwrap();
    for i in 0u16..256 {
        let value = ((i * 5) >> 8) as u8;
        writeln!(color_file, "    {value},").unwrap();
    }
    writeln!(color_file, "];").unwrap();
    writeln!(color_file).unwrap();

    writeln!(color_file, "pub(crate) const GRAY_CODES: [u8; 256] = [").unwrap();
    for i in 0u16..256 {
        let value = i as u8;
        let code = if value < 8 {
            16
        } else if value > 248 {
            231
        } else {
            232 + ((((value as u16 - 8) * 25) >> 8) as u8)
        };
        writeln!(color_file, "    {code},").unwrap();
    }
    writeln!(color_file, "];").unwrap();

    drop(color_file);

    println!("cargo:rerun-if-changed=build.rs");
}
