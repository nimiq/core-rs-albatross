# BLS
This is a Rust crate for making Boneh-Lynn-Shacham signatures. It has functionality for:

* Securely creating key pairs
* Signing messages
* Verifying signatures
* Aggregating signatures on the same message
* Serializing compressed versions of both public keys and signatures

It only supports the MNT6-753 elliptic curve. This curve was chosen to allow the creation of SNARKs proving statements
about these BLS signatures. These SNARKs can be created using the library [ZEXE](https://github.com/scipr-lab/zexe) and
are a fundamental part of Nimiq's Nano nodes (implemented on the nano-sync crate).

It uses only the algebra and algebra-core crates in [ZEXE](https://github.com/scipr-lab/zexe) for its elliptic curve
arithmetic.

## Usage
Bring the `nimiq-bls` crate into your project just as you normally would.

### Simple signatures
```rust
// Generate a random keypair. Make sure to provide a secure RNG.
let keypair = KeyPair::generate(rng);

// Create a message.
let message = "Some message";

// Sign the message. This function already hashes the message using Blake2s. 
let signature = keypair.sign(&message);

// Verify the signature against a given keypair. You can also do it using
// only the public key instead of the entire keypair.
assert!(keypair.verify(&message, &signature));
```

### Aggregate signatures
```rust
// Create a message.
let message = "Same message";

let mut public_keys = Vec::new();

let mut signatures = Vec::new();

for _ in 0..100 {
    // Generate a random keypair. Make sure to provide a secure RNG.
    let keypair = KeyPair::generate(rng);

    // Sign the message. This function already hashes the message using Blake2s.
    let signature = keypair.sign(&message);

    public_keys.push(keypair.public_key);

    signatures.push(signature);
}

// Create an aggregated public key from individual public keys. This function simply
// adds all the public keys together.
let agg_public_key = AggregatePublicKey::from_public_keys(&public_keys);

// Create an aggregated signature from individual signatures. This function simply
// adds all the signatures together.
let agg_signature = AggregateSignature::from_signatures(&signatures);

// Verify the aggregated signature against a given aggregated public key.
assert!(agg_public_key.verify(&message, &agg_signature));
```

## Security Warnings
This library does not make any guarantees about constant-time operations, memory access patterns, or resistance to
side-channel attacks.

**Important Note:** When aggregating signatures of the same message, please do read Section 3.2 of
[this paper](https://crypto.stanford.edu/~dabo/pubs/papers/aggreg.pdf). An adversary that is able to choose arbitrary
keys can forge signatures by publishing public keys it does not know the secret key for. This is known as a "Rogue Key
Attack".

One countermeasure is to require the adversary to prove knowledge of the corresponding secret key. This can be done by,
for example, publishing a signature of its own public key.

The aggregation of signatures of distinct messages is not vulnerable to the above mentioned attack. So, another
countermeasure is for the signer to implicitly append its public key to the message, hence creating distinct
messages. However, this library does not implement the aggregation of signatures for different messages.

## License
Licensed under either of:

 * Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

### Contribution
Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.
