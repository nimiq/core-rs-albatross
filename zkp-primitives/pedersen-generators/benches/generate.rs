use bencher::{benchmark_group, benchmark_main, Bencher};

pub fn bench_generate(bench: &mut Bencher) {
    bench.iter(nimiq_pedersen_generators::default_mnt6);
}

benchmark_group!(benches, bench_generate);
benchmark_main!(benches);
