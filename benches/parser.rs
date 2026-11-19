use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn parser_benchmark(c: &mut Criterion) {
    let source = include_str!("testdata/events.zig");

    c.bench_function("parser", |b| {
        b.iter(|| {
            black_box(zigsyn::parse_source(black_box(source)).unwrap());
        });
    });
}

criterion_group!(benches, parser_benchmark);
criterion_main!(benches);
