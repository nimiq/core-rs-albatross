# bls

This is a Rust crate for making Boneh-Lynn-Shacham signatures. It only supports the BLS12-377 curve. This curve was chosen to allow the creation of SNARKs of statements about these BLS signatures. These SNARKs can be created using [ZEXE](https://github.com/scipr-lab/zexe).

It mainly uses the Algebra crate in [ZEXE](https://github.com/scipr-lab/zexe) for its elliptic curve arithmetic.

## Usage

Bring the `nimiq-bls` crate into your project just as you normally would.

### Simple signatures

```rust
let keypair = KeyPair::generate(&mut rng);
let message = "Some message";
let sig = keypair.sign(&message.as_bytes());
assert!(keypair.verify(&message.as_bytes(), &sig));
```

### Aggregate signatures

```rust
let mut inputs = Vec::new();
let mut agg_sig = AggregateSignature::new();

let keypair1 = Keypair::generate(&mut rng);
let message1 = "Some message";
let sig1 = keypair1.sign(&message1.as_bytes());
inputs.push((keypair1.public, message1));
agg_sig.aggregate(&sig1);

let keypair2 = Keypair::generate(&mut rng);
let message2 = "Another message";
let sig2 = keypair2.sign(&message2.as_bytes());
inputs.push((keypair2.public, message2));
agg_sig.aggregate(&sig2);

assert!(
    agg_sig.verify(&inputs.iter()
        .map(|&(ref pk, ref m)| (pk, m.as_bytes()))
        .collect())
);
```

## Security Warnings

This library does not make any guarantees about constant-time operations, memory access patterns, or resistance to side-channel attacks.

**Important Note:** When aggregating signatures of the same message, please do read Section 3.2 of [this paper](https://crypto.stanford.edu/~dabo/pubs/papers/aggreg.pdf).
An adversary that is able to choose arbitrary keys can forge signatures by publishing public keys it does not know the secret key for. This is known as a "Rogue Key Attack".

Thus, one countermeasure is to require the adversary to prove knowledge of the secret key. This can be done by, for example, publishing a signature of his/her own public key.

The aggregation of signatures of distinct messages is not vulnerable to the above mentioned attack. So, another countermeasure is for the signer to implicitly append his/her public key to the message, hence creating distinct messages. However, this library does not implement the aggregation of signatures for different messages.

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
