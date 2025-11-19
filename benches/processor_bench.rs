use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use lolcat_ultra::{ColorMode, Config, process_input_with_color_mode};
use std::io::{BufReader, Cursor, Write};

// Sink writer that discards all output (for pure processing benchmarks)
struct Sink;

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// Test input generators
fn generate_ascii_lines(num_lines: usize, line_length: usize) -> String {
    let line: String = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. "
        .chars()
        .cycle()
        .take(line_length)
        .collect();

    std::iter::repeat(line)
        .take(num_lines)
        .collect::<Vec<_>>()
        .join("\n")
}

fn generate_unicode_lines(num_lines: usize, line_length: usize) -> String {
    let line: String = "Hello 世界 🌈 Привет مرحبا こんにちは "
        .chars()
        .cycle()
        .take(line_length)
        .collect();

    std::iter::repeat(line)
        .take(num_lines)
        .collect::<Vec<_>>()
        .join("\n")
}

fn generate_mixed_content() -> String {
    let mut content = String::new();

    // ASCII text
    content.push_str("The quick brown fox jumps over the lazy dog.\n");
    content.push_str("ABCDEFGHIJKLMNOPQRSTUVWXYZ\n");
    content.push_str("0123456789 !@#$%^&*()_+-=[]{}\\|;:'\",.<>?/\n");

    // Unicode text
    content.push_str("Unicode: 世界 🌈 Привет مرحبا こんにちは\n");
    content.push_str("Emojis: 😀 🎉 ✨ 🚀 💻 🔥\n");

    // Tabs and spaces
    content.push_str("Tabs:\t\t\tand\t\t\tspaces\n");

    // Repeat for reasonable size
    content.repeat(20)
}

fn bench_process_truecolor(c: &mut Criterion) {
    let mut group = c.benchmark_group("process_truecolor");

    for &(num_lines, line_len) in &[(10, 80), (100, 80), (1000, 80)] {
        let input = generate_ascii_lines(num_lines, line_len);
        let size = input.len();

        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}lines_{}chars", num_lines, line_len)),
            &input,
            |b, input| {
                b.iter(|| {
                    let reader = BufReader::new(Cursor::new(input.as_bytes()));
                    let writer = Sink;
                    let config = Config::try_new(0.1, 3.0, false).unwrap();

                    // Write to sink to avoid I/O overhead in benchmark
                    let result = process_input_with_color_mode(
                        reader,
                        writer,
                        black_box(&config),
                        ColorMode::TrueColor,
                    );

                    result.unwrap()
                });
            },
        );
    }

    group.finish();
}

fn bench_process_256color(c: &mut Criterion) {
    let mut group = c.benchmark_group("process_256color");

    for &(num_lines, line_len) in &[(10, 80), (100, 80), (1000, 80)] {
        let input = generate_ascii_lines(num_lines, line_len);
        let size = input.len();

        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{}lines_{}chars", num_lines, line_len)),
            &input,
            |b, input| {
                b.iter(|| {
                    let reader = BufReader::new(Cursor::new(input.as_bytes()));
                    let writer = Sink;
                    let config = Config::try_new(0.1, 3.0, false).unwrap();

                    // Write to sink to avoid I/O overhead in benchmark
                    let result = process_input_with_color_mode(
                        reader,
                        writer,
                        black_box(&config),
                        ColorMode::Color256,
                    );

                    result.unwrap()
                });
            },
        );
    }

    group.finish();
}

fn bench_process_unicode(c: &mut Criterion) {
    let mut group = c.benchmark_group("process_unicode");

    let input = generate_unicode_lines(100, 50);
    let size = input.len();

    group.throughput(Throughput::Bytes(size as u64));
    group.bench_function("unicode_truecolor", |b| {
        b.iter(|| {
            let reader = BufReader::new(Cursor::new(input.as_bytes()));
            let writer = Sink;
            let config = Config::try_new(0.1, 3.0, false).unwrap();

            let result = process_input_with_color_mode(
                reader,
                writer,
                black_box(&config),
                ColorMode::TrueColor,
            );

            result.unwrap()
        });
    });

    group.finish();
}

fn bench_process_mixed(c: &mut Criterion) {
    let mut group = c.benchmark_group("process_mixed_content");

    let input = generate_mixed_content();
    let size = input.len();

    group.throughput(Throughput::Bytes(size as u64));

    group.bench_function("mixed_truecolor", |b| {
        b.iter(|| {
            let reader = BufReader::new(Cursor::new(input.as_bytes()));
            let writer = Sink;
            let config = Config::try_new(0.1, 3.0, false).unwrap();

            let result = process_input_with_color_mode(
                reader,
                writer,
                black_box(&config),
                ColorMode::TrueColor,
            );

            result.unwrap()
        });
    });

    group.bench_function("mixed_256color", |b| {
        b.iter(|| {
            let reader = BufReader::new(Cursor::new(input.as_bytes()));
            let writer = Sink;
            let config = Config::try_new(0.1, 3.0, false).unwrap();

            let result = process_input_with_color_mode(
                reader,
                writer,
                black_box(&config),
                ColorMode::Color256,
            );

            result.unwrap()
        });
    });

    group.finish();
}

fn bench_process_slow_change(c: &mut Criterion) {
    let mut group = c.benchmark_group("process_slow_change");

    let input = generate_ascii_lines(1000, 80);
    let size = input.len();

    group.throughput(Throughput::Bytes(size as u64));
    group.bench_function("slow_truecolor", |b| {
        b.iter(|| {
            let reader = BufReader::new(Cursor::new(input.as_bytes()));
            let writer = Sink;
            // Very low frequency and high spread = color stays same for many chars
            let config = Config::try_new(0.001, 10.0, false).unwrap();

            let result = process_input_with_color_mode(
                reader,
                writer,
                black_box(&config),
                ColorMode::TrueColor,
            );

            result.unwrap()
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_process_truecolor,
    bench_process_256color,
    bench_process_unicode,
    bench_process_mixed,
    bench_process_slow_change
);
criterion_main!(benches);
