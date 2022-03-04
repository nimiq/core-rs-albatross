use criterion::{black_box, criterion_group, criterion_main, Criterion};

use nimiq_hash::{Blake2bHasher, Blake2sHasher, Hasher, Sha256Hasher};

pub fn bench(c: &mut Criterion) {
    let bytes = b"01234567890123456789012345678901";
    c.bench_function("blake2b short", |b| {
        b.iter(|| Blake2bHasher::default().digest(black_box(bytes)))
    });
    c.bench_function("blake2s short", |b| {
        b.iter(|| Blake2sHasher::default().digest(black_box(bytes)))
    });
    c.bench_function("sha256 short", |b| {
        b.iter(|| Sha256Hasher::default().digest(black_box(bytes)))
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
