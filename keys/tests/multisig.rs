use curve25519_dalek::{edwards::EdwardsPoint, scalar::Scalar};
use hex::FromHex;
use nimiq_hash::Blake2bHasher;
use nimiq_keys::{
    multisig::{
        address::{combine_public_keys, compute_address},
        commitment::{Commitment, CommitmentPair, Nonce},
        partial_signature::PartialSignature,
        CommitmentsBuilder,
    },
    Address, Ed25519PublicKey, KeyPair, PrivateKey,
};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_test_log::test;
use nimiq_test_utils::test_rng;
use nimiq_utils::merkle::MerklePath;
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
                "cannot verify {i}"
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

#[test]
fn it_can_create_a_valid_multisignature() {
    let keypair_a = "14a3bc3b25c73b6ca3e829aef329a2a6dc69ae52b8d20a164831a021b6a9f9feec98d39d98a58c13d399673d6da7dc6c74f379eddd8c8628e40ffc6be7c2498300";
    let keypair_b = "2da15ede9992fad834b73283dd1a24f5a7a52b067b09be132ddb5232df863125bb639b6bbf6db003a94a83ef9d12f12fcc5990f63954b7f6d88f5be58f8c411200";
    let keypair_c = "a3b3d799e7fca4baa3568d58e0c909af1f832926020163a1d48998621a15c9c6b81b12bcb1a6e9ba49a6dec268705c2cc2d70d1d7e22493a4128559eadacdbd400";
    let wallet_address = "NQ77 XKHG BUSE L76F 030F FY5U 0C6H 6HXU BSPX";

    let commitment_pair_a_1 = "61514436ba3671457a39ab8b89c166a6dbf9dcf2320142412faca62c0e30180ec441e06b23ef64095dd24ba9976e1bd6086dd34f6d2892ec92c8f3a5365e352f";
    let commitment_pair_a_2 = "246a60bacd6be35bc248de42bd8d8035c66766af037859797a3c6c87475fc20a6af6931e2199aa73707d1e2363502af6a637a33ddc9464b5a60dab9c5535240d";

    let commitment_pair_b_1 = "1c25176a8d9531dfdabd393e24457ef768b8f91ad1aa5b5c5d531c59d61493068bcf3923fe74da2c0dae83a0f0a4ad78c3ace4737e1bab09ae839059cc06b75a";
    let commitment_pair_b_2 = "9d372fe33120b7555f06112efa51a179e745ae03cc0942319a0b2a605c680708170b6a773e7f633ef7c3830ebe16a4a7dde24ba4040c18b361b6aa5fad2d0e6f";

    let signing_public_key = "768aa1e50751d31c7e16708903f6906621da68fd1daae12210480dac10d8a57b";
    let aggregate_commitment = "f52764eed6c6f89f6a07781f035707aab471e846a9b815c73d0eb620cf345b82";
    let b_scalar = "9556574afd98401d38bee97e9ecc70a3d0d29f3221fd4b73a71f6444678b3306";

    let partial_signature_a = "0f751ab3db73576994159919a970b529a68e0c1f5b49501243c573106cff200d";
    let partial_signature_b = "db2cd118d2e46fed51cff0341a139b90159a134095d7a003671d71d53a33520c";

    let signed_transaction = "01f4e305f34ea1ccf00c0f7fcbc030d1347dc5eafe000000000000000000000000000000000000000000000000000000000000000a0000000000000000000000000500a50100768aa1e50751d31c7e16708903f6906621da68fd1daae12210480dac10d8a57b02018002ff2353719738df451db5eaa049f2b8c95493c34b008aa4d9d452e6820bec66034b43399405dfc64024a1b9ff974fb9b1f428461d28b7d262c16d1dc5d8894542f52764eed6c6f89f6a07781f035707aab471e846a9b815c73d0eb620cf345b82fdcdf56e93f5b4fe0f4892abe48971a5bb28205ff020f115aae2e4e5a6327309";

    let keypair_a = KeyPair::deserialize_from_vec(&hex::decode(keypair_a).unwrap()).unwrap();
    let keypair_b = KeyPair::deserialize_from_vec(&hex::decode(keypair_b).unwrap()).unwrap();
    let keypair_c = KeyPair::deserialize_from_vec(&hex::decode(keypair_c).unwrap()).unwrap();

    let wallet_address = Address::from_user_friendly_address(wallet_address).unwrap();

    let commitment_pair_a_1 =
        CommitmentPair::deserialize_from_vec(&hex::decode(commitment_pair_a_1).unwrap()).unwrap();
    let commitment_pair_a_2 =
        CommitmentPair::deserialize_from_vec(&hex::decode(commitment_pair_a_2).unwrap()).unwrap();

    let commitment_pair_b_1 =
        CommitmentPair::deserialize_from_vec(&hex::decode(commitment_pair_b_1).unwrap()).unwrap();
    let commitment_pair_b_2 =
        CommitmentPair::deserialize_from_vec(&hex::decode(commitment_pair_b_2).unwrap()).unwrap();

    let signing_public_key = Ed25519PublicKey::from_hex(signing_public_key).unwrap();

    let combined_public_keys = combine_public_keys(
        vec![keypair_a.public, keypair_b.public, keypair_c.public],
        2,
    );
    assert_eq!(compute_address(&combined_public_keys), wallet_address);

    let mut tx = "01f4e305f34ea1ccf00c0f7fcbc030d1347dc5eafe000000000000000000000000000000000000000000000000000000000000000a0000000000000000000000000500".to_string();
    let tx_content = hex::decode("0000f4e305f34ea1ccf00c0f7fcbc030d1347dc5eafe00000000000000000000000000000000000000000000000000000000000a000000000000000000000000050000").unwrap();

    // Create partial signature A
    let commitments_data_a = CommitmentsBuilder::with_private_commitments(
        keypair_a.public,
        [commitment_pair_a_1, commitment_pair_a_2],
    )
    .with_signer(
        keypair_b.public,
        [
            commitment_pair_b_1.commitment(),
            commitment_pair_b_2.commitment(),
        ],
    )
    .build(&tx_content);

    assert_eq!(commitments_data_a.aggregate_public_key, signing_public_key);
    assert_eq!(
        hex::encode(commitments_data_a.aggregate_commitment.to_bytes()),
        aggregate_commitment
    );
    assert_eq!(hex::encode(commitments_data_a.b.to_bytes()), b_scalar);

    let signature_a = keypair_a
        .partial_sign(&commitments_data_a, &tx_content)
        .unwrap();
    assert_eq!(hex::encode(signature_a.0.to_bytes()), partial_signature_a);

    // Create partial signature B
    let commitments_data_b = CommitmentsBuilder::with_private_commitments(
        keypair_b.public,
        [commitment_pair_b_1, commitment_pair_b_2],
    )
    .with_signer(
        keypair_a.public,
        [
            commitment_pair_a_1.commitment(),
            commitment_pair_a_2.commitment(),
        ],
    )
    .build(&tx_content);

    assert_eq!(commitments_data_b.aggregate_public_key, signing_public_key);
    assert_eq!(
        hex::encode(commitments_data_b.aggregate_commitment.to_bytes()),
        aggregate_commitment
    );
    assert_eq!(hex::encode(commitments_data_b.b.to_bytes()), b_scalar);

    let signature_b = keypair_b
        .partial_sign(&commitments_data_b, &tx_content)
        .unwrap();
    assert_eq!(hex::encode(signature_b.0.to_bytes()), partial_signature_b);

    // Construct proof
    let mut proof = Vec::new();
    proof.push(0); // Signature proof type and flags
    proof.extend_from_slice(&commitments_data_a.aggregate_public_key.serialize_to_vec());

    let merkle_path = MerklePath::new::<Blake2bHasher, Ed25519PublicKey>(
        &combined_public_keys,
        &commitments_data_a.aggregate_public_key,
    );
    proof.extend_from_slice(&merkle_path.serialize_to_vec());

    let combined_signature = signature_a + signature_b;
    let signature = combined_signature.to_signature(&commitments_data_a.aggregate_commitment);
    proof.extend_from_slice(&signature.serialize_to_vec());

    assert_eq!(proof.len(), 165);

    // Add proof to transaction
    tx.push_str("a501"); // 165 encoded as varint
    tx.push_str(&hex::encode(proof));

    assert_eq!(&tx, signed_transaction);
}
