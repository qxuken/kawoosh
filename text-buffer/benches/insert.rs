use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use text_buffer::Buffer;

const TYPEWRITER_ROUNDS: usize = 10_000;

fn typing_byte(index: usize) -> u8 {
    b'a' + (index % 26) as u8
}

fn bench_typing_sequential(c: &mut Criterion) {
    let mut group = c.benchmark_group("typing_sequential");

    group.bench_function("insert_char", |b| {
        b.iter(|| {
            let mut buffer = Buffer::new();
            buffer.set_text(black_box(b"some initial text\n"));

            for index in 0..TYPEWRITER_ROUNDS {
                let offset = buffer.len();
                buffer.insert_char(offset, black_box(typing_byte(index)));
            }

            black_box(buffer.len())
        });
    });

    group.bench_function("insert_slice", |b| {
        b.iter(|| {
            let mut buffer = Buffer::new();
            buffer.set_text(black_box(b"some initial text\n"));

            for index in 0..TYPEWRITER_ROUNDS {
                let offset = buffer.len();
                buffer.insert(offset, black_box(&[typing_byte(index)]));
            }

            black_box(buffer.len())
        });
    });

    group.finish();
}

/// Force the persistent slow path on every keystroke by keeping a fresh
/// snapshot alive, approximating the pre-fast-path cost of `insert`.
fn bench_typing_forced_fallback(c: &mut Criterion) {
    let mut group = c.benchmark_group("typing_forced_fallback");

    group.bench_function("insert_char", |b| {
        b.iter(|| {
            let mut buffer = Buffer::new();
            buffer.set_text(black_box(b"some initial text\n"));

            for index in 0..TYPEWRITER_ROUNDS {
                let _snapshot = buffer.clone();
                let offset = buffer.len();
                buffer.insert_char(offset, black_box(typing_byte(index)));
            }

            black_box(buffer.len())
        });
    });

    group.bench_function("insert_slice", |b| {
        b.iter(|| {
            let mut buffer = Buffer::new();
            buffer.set_text(black_box(b"some initial text\n"));

            for index in 0..TYPEWRITER_ROUNDS {
                let _snapshot = buffer.clone();
                let offset = buffer.len();
                buffer.insert(offset, black_box(&[typing_byte(index)]));
            }

            black_box(buffer.len())
        });
    });

    group.finish();
}

fn bench_insert_char_random_offsets(c: &mut Criterion) {
    let mut group = c.benchmark_group("insert_char_random_offsets");

    group.bench_function("insert_char", |b| {
        b.iter(|| {
            let mut buffer = Buffer::new();
            buffer.set_text(black_box(b"some initial text\n"));

            let mut seed = 0x9e37_79b9_7f4a_7c15u64;
            for _ in 0..TYPEWRITER_ROUNDS {
                seed = seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                let offset = (seed >> 33) as usize % (buffer.len() + 1);
                buffer.insert_char(offset, black_box(b'x'));
            }

            black_box(buffer.len())
        });
    });

    group.bench_function("insert_slice", |b| {
        b.iter(|| {
            let mut buffer = Buffer::new();
            buffer.set_text(black_box(b"some initial text\n"));

            let mut seed = 0x9e37_79b9_7f4a_7c15u64;
            for _ in 0..TYPEWRITER_ROUNDS {
                seed = seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                let offset = (seed >> 33) as usize % (buffer.len() + 1);
                buffer.insert(offset, black_box(b"x"));
            }

            black_box(buffer.len())
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_typing_sequential,
    bench_typing_forced_fallback,
    bench_insert_char_random_offsets
);
criterion_main!(benches);
