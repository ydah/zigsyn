use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn scanner_benchmark(c: &mut Criterion) {
    let source = include_str!("testdata/events.zig");

    c.bench_function("scanner", |b| {
        b.iter(|| {
            let scanner = zigsyn::scanner::Scanner::from(black_box(source));
            black_box(scanner.scan_all().unwrap());
        });
    });
}

criterion_group!(benches, scanner_benchmark);
criterion_main!(benches);
