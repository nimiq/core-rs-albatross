use criterion::{black_box, criterion_group, criterion_main, Criterion};
use nimiq_hash::Blake2bHash;
use postcard;

fn serialize_hash(c: &mut Criterion) {
    let hash = Blake2bHash::default();
    let mut buf = [0; 32];
    let mut group = c.benchmark_group("serialize_hash");
    group.bench_function("serde", |b| {
        b.iter(|| {
            let _ = black_box(postcard::to_slice(black_box(&hash), &mut buf).unwrap());
        })
    });
    group.bench_function("plain", |b| {
        b.iter(|| {
            let _ = black_box(black_box(&hash).serialize(&mut buf.as_mut_slice()).unwrap());
        })
    });
    group.finish();
}

criterion_group!(hash, serialize_hash,);
criterion_main!(hash);
