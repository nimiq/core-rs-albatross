# bls

This is a Rust crate for making BLS (Boneh-Lynn-Shacham) signatures. It currently supports the [BLS12-381](https://z.cash/blog/new-snark-curve.html) (Barreto-Lynn-Scott) (yes, I know) construction.

## Documentation

Bring the `bls` crate into your project just as you normally would.

```rust
use bls::Keypair;
use pairing::bls12_381::Bls12;

let keypair = Keypair::<Bls12>::generate(&mut rng);
let message = "Some message";
let sig = keypair.sign(&message.as_bytes());
assert_eq!(keypair.verify(&message.as_bytes(), &sig), true);
```

### Aggregate signatures

```rust
use bls::{AggregateSignature, Keypair};
use pairing::bls12_381::Bls12;

let mut inputs = Vec::new();
let mut asig = AggregateSignature::new();

let keypair1 = Keypair::<Bls12>::generate(&mut rng);
let message1 = "Some unique message";
let sig1 = keypair1.sign(&message1.as_bytes());
inputs.push((keypair1.public, message1));
asig.aggregate(&sig1);

let keypair2 = Keypair::<Bls12>::generate(&mut rng);
let message2 = "Some other unique message";
let sig2 = keypair2.sign(&message2.as_bytes());
inputs.push((keypair2.public, message2));
asig.aggregate(&sig2);

assert_eq!(
    asig.verify(&inputs.iter()
        .map(|&(ref pk, ref m)| (pk, m.as_bytes()))
        .collect()),
    true
);
```

**Important:** When aggregating signatures of the same message, please do read Section 3.2 of [this paper](https://crypto.stanford.edu/~dabo/pubs/papers/aggreg.pdf).
An adversary that is able to choose arbitrary keys can forge signatures by publishing public keys it does not know the secret key for.
Thus, one countermeasure is to require the adversary to prove knowledge of the discrete logarithms (to base G2) of his published public keys.
Alternatively, the signer implicitly prepends his/her public key to the message and hence creates distinct messages.
Aggregating distinct messages only suffices to prevent the mentioned attack.

## Security Warnings

This library does not make any guarantees about constant-time operations, memory access patterns, or resistance to side-channel attacks.

## License

Licensed under either of

 * Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.

