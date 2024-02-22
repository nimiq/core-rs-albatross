use curve25519_dalek::{edwards::EdwardsPoint, scalar::Scalar};
use nimiq_keys::{
    multisig::{
        commitment::{Commitment, CommitmentPair, Nonce},
        partial_signature::PartialSignature,
        CommitmentsBuilder,
    },
    Ed25519PublicKey, KeyPair, PrivateKey,
};
use nimiq_test_log::test;
use nimiq_test_utils::test_rng::test_rng;
use sha2::Digest;

struct StrTestVector {
    priv_keys: &'static [&'static str],
    message: &'static str,
}

struct TestVector {
    priv_keys: Vec<PrivateKey>,
    message: Vec<u8>,
}

macro_rules! from_hex {
    ($hex: expr, $len: expr, $call: path) => {{
        $call(from_hex!($hex, $len))
    }};
    ($hex: expr, $len: expr) => {{
        let bytes = hex::decode($hex).unwrap();
        let mut fixed_bytes = [0u8; $len];
        fixed_bytes.copy_from_slice(bytes.as_slice());
        fixed_bytes
    }};
}

macro_rules! from_hex_vec {
    ($vec: expr, $len: expr, $call: path) => {{
        $vec.iter()
            .map(|item| from_hex!(item, $len, $call))
            .collect()
    }};
    ($vec: expr, $len: expr) => {{
        $vec.iter().map(|item| from_hex!(item, $len))
    }};
}

impl TestVector {
    fn from_str(v: &StrTestVector) -> TestVector {
        let priv_keys: Vec<PrivateKey> =
            from_hex_vec!(v.priv_keys, PrivateKey::SIZE, PrivateKey::from);
        let message: Vec<u8> = v.message.to_string().into_bytes();
        TestVector { priv_keys, message }
    }
}

const VECTORS: [StrTestVector; 4] = [
    StrTestVector {
        priv_keys: &[
            "f80793b4cb1e165d1a65b5cbc9e7b2efa583de01bc13dd23f7a1d78af4349904",
            "407fd16dd5e908ea81f755a1fb2591dc7b19c2efcfeb517273aa7c520a5d8c06",
        ],
        message: "",
    },
    StrTestVector {
        priv_keys: &[
            "af6cccd64c2679d6bdfac26f32ab8c2ad1b875ec3e1ab7933509218bcea69c0a",
            "459124f418cc9ac5e027886e0bf6591493263b984f3d5d7fdf17867d327b1a0e",
        ],
        message: "Hello World",
    },
    StrTestVector {
        priv_keys: &[
            "fce1ccbeefe33e3d25dc198f69a58a2c40f24a75d0ea728cef65ad7e309ac20e",
            "312a4339dfd85e9650e918b3a714196cd2e54b4c84ac6f7abc1db897c2f4aa0c",
            "726254985d2fd37ac244365465bb52049e60396051daeaff5be858ef2bff2105",
            "0c817fcffd7c0e90d1957257b10b617e448454a029cdf25bcb8e2fa312c7860d",
        ],
        message: "",
    },
    StrTestVector {
        priv_keys: &[
            "65edb8c173fdbaf5e106ca53069cde47c2a7627518228d8269a3da35f5fd0001",
            "574e47b97d5918ee12f3872c7843b789c34298ff7cc59ea049586afe63f7c20a",
            "a8177dccd4557044db70f3066960a1df283ba5f0b8ffa997c6411c9119ac160d",
            "21ea275ae38602ef65aac6774db8ed2e6164923b14a7e11c40df874f453b780a",
        ],
        message: "Hello World",
    },
];

#[test]
fn it_can_construct_public_keys() {
    for vector in VECTORS.iter() {
        let test = TestVector::from_str(vector);

        for i in 0..test.priv_keys.len() {
            let _public_key: Ed25519PublicKey = Ed25519PublicKey::from(&test.priv_keys[i]);
        }
    }
}

#[test]
fn it_correctly_calculates_commitments() {
    // Create random 32 bytes.
    let randomness: [u8; Nonce::SIZE] = [
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
        25, 26, 27, 28, 29, 30, 31,
    ];

    // Decompress the 32 byte cryptographically secure random data to 64 byte.
    let mut h: ::sha2::Sha512 = ::sha2::Sha512::default();

    h.update(randomness);
    let scalar = Scalar::from_hash::<::sha2::Sha512>(h);

    // Compute the point [scalar]B.
    let commitment: EdwardsPoint = &scalar * ::curve25519_dalek::constants::ED25519_BASEPOINT_TABLE;

    assert_eq!(
        scalar.as_bytes(),
        hex::decode("6ee2c0c33a62b1bd39f88528fb2daecbc8d54d69a31cbb32da758ac25a55a40f")
            .unwrap()
            .as_slice()
    );
    assert_eq!(
        commitment.compress().as_bytes(),
        hex::decode("b6d4f93caf5d574e9765db8740c956400c2d6532d179b0d87b4f6b79ba93a387")
            .unwrap()
            .as_slice()
    );
}

#[test]
fn it_can_create_signatures() {
    let mut rng = test_rng(true);
    for vector in VECTORS.iter() {
        let test = TestVector::from_str(vector);

        let mut pks = vec![];
        let mut commitment_pairs = vec![];
        let mut commitments = vec![];
        let mut partial_sigs = vec![];

        for i in 0..test.priv_keys.len() {
            let key_pair = KeyPair::from(test.priv_keys[i].clone());
            let pair = CommitmentPair::generate_all(&mut rng);
            pks.push(key_pair.public);
            commitments.push(CommitmentPair::to_commitments(&pair));
            commitment_pairs.push(pair);
        }

        let mut agg_commitment = Commitment::default();
        let mut agg_pk = Ed25519PublicKey::default();
        for i in 0..test.priv_keys.len() {
            let key_pair = KeyPair::from(test.priv_keys[i].clone());
            let mut builder =
                CommitmentsBuilder::with_private_commitments(key_pair.public, commitment_pairs[i]);
            for j in 0..test.priv_keys.len() {
                if i != j {
                    builder = builder.with_signer(pks[j], commitments[j]);
                }
            }
            let data = builder.build(&test.message);
            let partial_sig = key_pair.partial_sign(&data, &test.message).unwrap();

            assert!(
                key_pair
                    .public
                    .verify_partial(&data, &partial_sig, &test.message),
                "cannot verify {}",
                i
            );

            if i > 0 {
                assert_eq!(agg_commitment, data.aggregate_commitment);
                assert_eq!(agg_pk, data.aggregate_public_key);
            }
            agg_commitment = data.aggregate_commitment;
            agg_pk = data.aggregate_public_key;

            partial_sigs.push(partial_sig);
        }

        let sig = partial_sigs
            .iter()
            .sum::<PartialSignature>()
            .to_signature(&agg_commitment);

        assert!(agg_pk.verify(&sig, &test.message));
    }
}
