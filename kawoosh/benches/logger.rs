//! What a `log::` call costs its caller with the notification sink
//! installed (`logger.rs`): the cut for a record nobody wants, a
//! literal message, a formatted one — each with a drain on another
//! thread, as the frame is.

use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use criterion::{Criterion, criterion_group, criterion_main};
use kawoosh::logger::Logger;
use kawoosh_systems::WakeHandle;
use log::Log;

fn bench_log(c: &mut Criterion) {
    let wake = WakeHandle::new();
    wake.set(Arc::new(|| {}));
    let (logger, sink) = Logger::new(wake);
    // The frame's side: drains as fast as it can, so the channel never
    // grows without bound.
    let stop = Arc::new(AtomicBool::new(false));
    let drain = {
        let stop = stop.clone();
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                for r in sink.drain() {
                    black_box(r);
                }
                std::thread::yield_now();
            }
        })
    };
    let mut group = c.benchmark_group("log");
    group.bench_function("filtered_out", |b| {
        b.iter(|| {
            logger.log(
                &log::Record::builder()
                    .level(log::Level::Debug)
                    .target(black_box("wgpu_core::device"))
                    .args(format_args!("resource {} created", black_box(7)))
                    .build(),
            )
        })
    });
    group.bench_function("literal", |b| {
        b.iter(|| {
            logger.log(
                &log::Record::builder()
                    .level(log::Level::Debug)
                    .target(black_box("kawoosh_systems::lsp"))
                    .args(format_args!("didChange sent"))
                    .build(),
            )
        })
    });
    group.bench_function("formatted", |b| {
        b.iter(|| {
            logger.log(
                &log::Record::builder()
                    .level(log::Level::Debug)
                    .target(black_box("kawoosh_systems::lsp"))
                    .args(format_args!(
                        "didChange sent for {} at {}",
                        black_box("src/main.rs"),
                        black_box(42)
                    ))
                    .build(),
            )
        })
    });
    group.finish();
    stop.store(true, Ordering::Relaxed);
    let _ = drain.join();
}

criterion_group!(benches, bench_log);
criterion_main!(benches);
