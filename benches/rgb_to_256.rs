use criterion::{Criterion, black_box, criterion_group, criterion_main};
use lolcat_ultra::color::rgb_to_256;

fn make_dataset() -> Vec<(u8, u8, u8)> {
    let mut data = Vec::with_capacity(16 * 16 * 16);
    let mut r = 0u8;
    while r <= 240 {
        let mut g = 0u8;
        while g <= 240 {
            let mut b = 0u8;
            while b <= 240 {
                data.push((r, g, b));
                if b == 240 {
                    break;
                }
                b += 15;
            }
            if g == 240 {
                break;
            }
            g += 15;
        }
        if r == 240 {
            break;
        }
        r += 15;
    }
    data
}

fn bench_rgb_to_256(c: &mut Criterion) {
    let dataset = make_dataset();
    c.bench_function("rgb_to_256", |b| {
        b.iter(|| {
            for &(r, g, b) in &dataset {
                black_box(rgb_to_256(r, g, b));
            }
        });
    });
}

criterion_group!(benches, bench_rgb_to_256);
criterion_main!(benches);
