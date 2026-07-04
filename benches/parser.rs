use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn parser_benchmark(c: &mut Criterion) {
    let source = r#"
//! docs
pub fn main() void {
    const answer: u32 = 42;
    _ = @import("std");
}
"#;

    c.bench_function("parser", |b| {
        b.iter(|| {
            black_box(zigsyn::parse_source(black_box(source)).unwrap());
        });
    });
}

criterion_group!(benches, parser_benchmark);
criterion_main!(benches);
