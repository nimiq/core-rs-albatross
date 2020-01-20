use lazy_static::lazy_static;

use std::str::FromStr;
use criterion::{Criterion, Benchmark, criterion_group, criterion_main};
use fixed_unsigned::types::FixedUnsigned10;
use bigdecimal::BigDecimal;


const NUM_1: &'static str = "9127612783.1287512387";
const NUM_2: &'static str = "1021235791.2340980123";

// get numbers at fixed precision. We want to have exactly 10 decimal places. This won't do it
// exactly, but I reckon changing the precision will have the same impact, regardless of by how
// much. Should be correct +- 1 digit anyway.
const PRECISION: u64 = 20;


lazy_static! {
    pub static ref FIXED_1: FixedUnsigned10 = FixedUnsigned10::from_str(NUM_1).unwrap();
    pub static ref FIXED_2: FixedUnsigned10 = FixedUnsigned10::from_str(NUM_2).unwrap();
    pub static ref BIGDECIMAL_1: BigDecimal = BigDecimal::from_str(NUM_1).unwrap();
    pub static ref BIGDECIMAL_2: BigDecimal = BigDecimal::from_str(NUM_2).unwrap();
}



fn criterion_benchmark(c: &mut Criterion) {
    c.bench(
        "from_str",
        Benchmark::new("FixedUnsigned", |b| b.iter(|| FixedUnsigned10::from_str(NUM_1).unwrap()))
            .with_function("BigDecimal", |b| b.iter(|| BigDecimal::from_str(NUM_1).unwrap()))
            .with_function("BigDecimal::with_prec", |b| b.iter(|| BigDecimal::from_str(NUM_1).unwrap().with_prec(PRECISION)))
    );
    c.bench(
        "to_str",
        Benchmark::new("FixedUnsigned", |b| b.iter(|| FIXED_1.to_string()))
            .with_function("BigDecimal", |b| b.iter(|| BIGDECIMAL_1.to_string()))
    );

    c.bench(
        "add",
        Benchmark::new("FixedUnsigned", |b| b.iter(|| &*FIXED_1 + &*FIXED_2))
            .with_function("BigDecimal", |b| b.iter(|| &*BIGDECIMAL_1 + &*BIGDECIMAL_2))
            .with_function("BigDecimal::with_prec", |b| b.iter(|| (&*BIGDECIMAL_1 + &*BIGDECIMAL_2).with_prec(PRECISION)))
    );
    c.bench(
        "sub",
        Benchmark::new("FixedUnsigned", |b| b.iter(|| &*FIXED_1 - &*FIXED_2))
            .with_function("BigDecimal", |b| b.iter(|| &*BIGDECIMAL_1 - &*BIGDECIMAL_2))
            .with_function("BigDecimal::with_prec", |b| b.iter(|| (&*BIGDECIMAL_1 - &*BIGDECIMAL_2).with_prec(PRECISION)))
    );
    c.bench(
        "mul",
        Benchmark::new("FixedUnsigned", |b| b.iter(|| &*FIXED_1 * &*FIXED_2))
            .with_function("BigDecimal", |b| b.iter(|| &*BIGDECIMAL_1 * &*BIGDECIMAL_2))
            .with_function("BigDecimal::with_prec", |b| b.iter(|| (&*BIGDECIMAL_1 * &*BIGDECIMAL_2).with_prec(PRECISION)))
    );
    c.bench(
        "div",
        Benchmark::new("FixedUnsigned", |b| b.iter(|| &*FIXED_1 / &*FIXED_2))
            .with_function("BigDecimal", |b| b.iter(|| &*BIGDECIMAL_1 / &*BIGDECIMAL_2))
            .with_function("BigDecimal::with_prec", |b| b.iter(|| (&*BIGDECIMAL_1 / &*BIGDECIMAL_2).with_prec(PRECISION)))
    );
}


criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);