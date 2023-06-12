#[macro_use]
extern crate bencher;

use ark_bls12_381::Bls12_381;
use ark_ec::{pairing::Pairing, Group};
use ark_mnt6_753::MNT6_753;
use ark_std::{UniformRand, Zero};
use bencher::Bencher;
use rand::thread_rng;

fn secret_key<C: Pairing>() -> C::ScalarField {
    let rng = &mut thread_rng();
    let mut sk = C::ScalarField::rand(rng);
    loop {
        if !sk.is_zero() {
            break;
        }
        sk = C::ScalarField::rand(rng);
    }
    sk
}

fn sign<C: Pairing>(sk: C::ScalarField, msg: C::G1) -> C::G1 {
    msg * sk
}

fn public_key<C: Pairing>(sk: C::ScalarField) -> C::G2 {
    C::G2::generator() * sk
}

fn verify<C: Pairing>(pk: C::G2, sig: C::G1, msg: C::G1) -> bool {
    let lhs = C::pairing(sig, C::G2::generator());
    let rhs = C::pairing(msg, pk);
    lhs == rhs
}

fn sign_mnt(bench: &mut Bencher) {
    let sk = secret_key::<MNT6_753>();
    let msg = <MNT6_753 as Pairing>::G1::rand(&mut thread_rng());

    bench.iter(|| {
        let _ = sign::<MNT6_753>(sk, msg);
    });
}

fn sign_bls12_381(bench: &mut Bencher) {
    let sk = secret_key::<Bls12_381>();
    let msg = <Bls12_381 as Pairing>::G1::rand(&mut thread_rng());

    bench.iter(|| {
        let _ = sign::<Bls12_381>(sk, msg);
    });
}

fn verify_mnt(bench: &mut Bencher) {
    let sk = secret_key::<MNT6_753>();
    let msg = <MNT6_753 as Pairing>::G1::rand(&mut thread_rng());
    let sig = sign::<MNT6_753>(sk, msg);
    let pk = public_key::<MNT6_753>(sk);

    bench.iter(|| {
        let _ = verify::<MNT6_753>(pk, sig, msg);
    });
}

fn verify_bls12_381(bench: &mut Bencher) {
    let sk = secret_key::<Bls12_381>();
    let msg = <Bls12_381 as Pairing>::G1::rand(&mut thread_rng());
    let sig = sign::<Bls12_381>(sk, msg);
    let pk = public_key::<Bls12_381>(sk);

    bench.iter(|| {
        let _ = verify::<Bls12_381>(pk, sig, msg);
    });
}

fn aggregate_mnt(bench: &mut Bencher) {
    let sk = secret_key::<MNT6_753>();
    let msg = <MNT6_753 as Pairing>::G1::rand(&mut thread_rng());
    let sig = sign::<MNT6_753>(sk, msg);
    let pk = public_key::<MNT6_753>(sk);

    bench.iter(|| {
        let _ = sig + sig;
        let _ = pk + pk;
    });
}

fn aggregate_bls12_381(bench: &mut Bencher) {
    let sk = secret_key::<Bls12_381>();
    let msg = <Bls12_381 as Pairing>::G1::rand(&mut thread_rng());
    let sig = sign::<Bls12_381>(sk, msg);
    let pk = public_key::<Bls12_381>(sk);

    bench.iter(|| {
        let _ = sig + sig;
        let _ = pk + pk;
    });
}

benchmark_group!(
    benches,
    sign_mnt,
    sign_bls12_381,
    verify_mnt,
    verify_bls12_381,
    aggregate_mnt,
    aggregate_bls12_381
);
benchmark_main!(benches);
