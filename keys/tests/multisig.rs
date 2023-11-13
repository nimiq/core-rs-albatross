use curve25519_dalek::{
    edwards::{CompressedEdwardsY, EdwardsPoint},
    scalar::Scalar,
};
use nimiq_keys::{
    multisig::{Commitment, PartialSignature, RandomSecret, *},
    Ed25519PublicKey, Ed25519Signature, KeyPair, PrivateKey,
};
use nimiq_test_log::test;
use sha2::Digest;

struct StrTestVector {
    priv_keys: &'static [&'static str],
    pub_keys: &'static [&'static str],
    pub_keys_hash: &'static str,
    delinearized_priv_keys: &'static [&'static str],
    delinearized_pub_keys: &'static [&'static str],
    secrets: &'static [&'static str],
    commitments: &'static [&'static str],
    agg_pub_key: &'static str,
    agg_commitment: &'static str,
    partial_signatures: &'static [&'static str],
    agg_signature: &'static str,
    signature: &'static str,
    message: &'static str,
}

struct TestVector {
    priv_keys: Vec<PrivateKey>,
    pub_keys: Vec<Ed25519PublicKey>,
    pub_keys_hash: [u8; 64],
    delinearized_priv_keys: Vec<Scalar>,
    delinearized_pub_keys: Vec<EdwardsPoint>,
    secrets: Vec<RandomSecret>,
    commitments: Vec<Commitment>,
    agg_pub_key: Ed25519PublicKey,
    agg_commitment: Commitment,
    partial_signatures: Vec<PartialSignature>,
    agg_signature: PartialSignature,
    signature: Ed25519Signature,
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
        let pub_keys: Vec<Ed25519PublicKey> =
            from_hex_vec!(v.pub_keys, Ed25519PublicKey::SIZE, Ed25519PublicKey::from);
        let pub_keys_hash: [u8; 64] = from_hex!(v.pub_keys_hash, 64);
        let delinearized_priv_keys: Vec<Scalar> = from_hex_vec!(
            v.delinearized_priv_keys,
            PrivateKey::SIZE,
            Scalar::from_bytes_mod_order
        );
        let delinearized_pub_keys: Vec<EdwardsPoint> =
            from_hex_vec!(v.delinearized_pub_keys, Ed25519PublicKey::SIZE)
                .map(|arr: [u8; 32]| CompressedEdwardsY(arr).decompress().unwrap())
                .collect();
        let secrets: Vec<RandomSecret> =
            from_hex_vec!(v.secrets, RandomSecret::SIZE, RandomSecret::from);
        let commitments: Vec<Commitment> = from_hex_vec!(v.commitments, Commitment::SIZE)
            .map(|arr: [u8; 32]| Commitment::from_bytes(arr).unwrap())
            .collect();
        let agg_pub_key: Ed25519PublicKey = from_hex!(
            v.agg_pub_key,
            Ed25519PublicKey::SIZE,
            Ed25519PublicKey::from
        );
        let agg_commitment: Commitment =
            Commitment::from_bytes(from_hex!(v.agg_commitment, Commitment::SIZE)).unwrap();
        let partial_signatures: Vec<PartialSignature> = from_hex_vec!(
            v.partial_signatures,
            PartialSignature::SIZE,
            PartialSignature::from
        );
        let agg_signature: PartialSignature = from_hex!(
            v.agg_signature,
            PartialSignature::SIZE,
            PartialSignature::from
        );
        let signature: Ed25519Signature =
            from_hex!(v.signature, Ed25519Signature::SIZE, Ed25519Signature::from);
        let message: Vec<u8> = v.message.to_string().into_bytes();
        TestVector {
            priv_keys,
            pub_keys,
            pub_keys_hash,
            delinearized_priv_keys,
            delinearized_pub_keys,
            secrets,
            commitments,
            agg_pub_key,
            agg_commitment,
            partial_signatures,
            agg_signature,
            signature,
            message,
        }
    }
}

const VECTORS: [StrTestVector; 4] = [
    StrTestVector {
        priv_keys: &[
            "f80793b4cb1e165d1a65b5cbc9e7b2efa583de01bc13dd23f7a1d78af4349904",
            "407fd16dd5e908ea81f755a1fb2591dc7b19c2efcfeb517273aa7c520a5d8c06",
        ],
        pub_keys: &[
            "8695ae0fc477e9c07138caa67d44b45b8d05bee777fd2a3d7c15cec726e4f010",
            "b14f7b1aea1125029453bc16e31488fec5ec5be75b371003610a6901f4035992",
        ],
        pub_keys_hash: "2f2e0d26bdb53fb8a4735a77251453eb06d6e9db6d67955adc8706336520fec40efdb7c43f396009a3560c04fdea75d810c55f11e8dbea562e0875438957e664",
        delinearized_priv_keys: &[
            "5531dc8d35f336bf3b5aadb03bf2369bb1cb0bdcd588ac6a5d1179370bfdf00a",
            "f4dd2f8c63d156f471e84af2a1964b03a0b58dafed33c02cc1a44b1b48eb820f",
        ],
        delinearized_pub_keys: &[
            "05750e281698ec0b6d2fb5252b7e474d647a5f4babc8d0befdcd04d5473a1579",
            "ef512dc2ea4583518ae1c9588d4d3655770f430f824fc36b21053abe7c7cb032",
        ],
        secrets: &[
            "8406d5a4872a124c834e1876ef518203ba343a318557d051d4d5035a82c20501",
            "33b2d49f68c1091470bf7723253af5a0e755a035f918708203804485b24b9e0d",
        ],
        commitments: &[
            "f3664d2921f41ea96ba8d2ea96f19a5844652bb746e65b2280da39f8a8a4a5fb",
            "5f5a38feced274ff07a094c271f45cc1eed70abdcc594fcb07999aeaf6c80307",
        ],
        agg_commitment: "5f6591c8d8a304fbdbb259826f8d88210c3aefce858f2432e91ba7224316861b",
        agg_pub_key: "5bf678b9604126cca1148cd3239178e989a91ac5a3fa78c64482c335360bb00b",
        partial_signatures: &[
            "65d2bb099b979b818fbd26428aa5dd54f8e3f1b71c2d3a3934b4338a9fe53708",
            "01eafa18d112094ff0888b3429b8bf52732adec4a52c327d26d5126a4b36d904",
        ],
        agg_signature: "66bcb6226caaa4d07f46b276b35d9da76b0ed07cc2596cb65a8946f4ea1b110d",
        signature: "5f6591c8d8a304fbdbb259826f8d88210c3aefce858f2432e91ba7224316861b66bcb6226caaa4d07f46b276b35d9da76b0ed07cc2596cb65a8946f4ea1b110d",
        message: "",
    },
    StrTestVector {
        priv_keys: &[
            "af6cccd64c2679d6bdfac26f32ab8c2ad1b875ec3e1ab7933509218bcea69c0a",
            "459124f418cc9ac5e027886e0bf6591493263b984f3d5d7fdf17867d327b1a0e",
        ],
        pub_keys: &[
            "73903ee860501452abc3dbf31ad484033f11e4ca99ecd99d42fc8eb2b0816b51",
            "198579bc339d5209fbc4bed6f8dcb6234d30d98b678fc5e0e44c9dfa58ddf73f",
        ],
        pub_keys_hash: "cc99ea463a1b146dcd0b83b95a15d6bfbe10907aaed1850cb5d85187e21ab37242d382f3f431bcdc33a5eb609ff5173a609b446b717e2c08e240e3be293da87a",
        delinearized_priv_keys: &[
            "a577441997f693a0c8463d0abe723d9974ae188c1a98cf33d72152864c5ca20c",
            "a7a3a56cc6b7e8f502d7317f3f79a41bb6fd06ab0793be5c539f5e44d5306706",
        ],
        delinearized_pub_keys: &[
            "39e5bb4ddaca066b4ce13cdfca93d3b46ccd9341e55e1f8b081a401dbd83dbd8",
            "314e065b8a2d7399d02a9bfedfc2a9514d87e3aadd08f3d35315662d803d510b",
        ],
        secrets: &[
            "a5ee1a93ce5bea1d20db30f12ec53ad4d3d87605dccb2abceb2dacaf54e8410d",
            "d4ac2ee84cf32bdad5a96c20590ad56fb4779e375fcd264ce8e9008be79b0b0d",
        ],
        commitments: &[
            "e4c4e8877d96548c49f8d395e4f990eff38108b03281eca62a3430d3500db7c4",
            "6c3d1be926e02a958717d6bc4934d541582cf1ab8fba19bce0d3c5226ebb69ac",
        ],
        agg_commitment: "bd7b7414e03273d3e23049586b0a97ed8164651937b17c2e52e2261a89fb4347",
        agg_pub_key: "08a2a7e9ed90811e8cabb48ffa2ff1a3b6e4f99bf81a552536cebbfed0a1ef8c",
        partial_signatures: &[
            "0fa57e95bbf4b5530d13a7e4dc405401c478b0e62a1a26319da966d13edaf105",
            "2d28a1db8447ee6fcb915a4d5a373b0dcdcedf1fab563a63c15c0ccee49e8505",
        ],
        agg_signature: "3ccd1f71403ca4c3d8a4013237788f0e91479006d67060945e06739f2379770b",
        signature: "bd7b7414e03273d3e23049586b0a97ed8164651937b17c2e52e2261a89fb43473ccd1f71403ca4c3d8a4013237788f0e91479006d67060945e06739f2379770b",
        message: "Hello World",
    },
    StrTestVector {
        priv_keys: &[
            "fce1ccbeefe33e3d25dc198f69a58a2c40f24a75d0ea728cef65ad7e309ac20e",
            "312a4339dfd85e9650e918b3a714196cd2e54b4c84ac6f7abc1db897c2f4aa0c",
            "726254985d2fd37ac244365465bb52049e60396051daeaff5be858ef2bff2105",
            "0c817fcffd7c0e90d1957257b10b617e448454a029cdf25bcb8e2fa312c7860d",
        ],
        pub_keys: &[
            "8e16f94ea50415097751606c7fd21e0bb51e340c3ba59f1a51dd430b8a8ef444",
            "94b8d238bbe70758ec7807d420dcbc0134b65fa9be03ca06592f957118f88010",
            "d365f216daf3f331ed2a803ec76968e0373f81695c4e8a70776a8f3eeba55a20",
            "f8bf28fe5d42a9a9dd0fa7a5826cd83a59c12181c366cba5eeefbc0c5b4fdac7",
        ],
        pub_keys_hash: "158910d4ed875025dd26f906dfde891e9a47c9586af60b5deca6ae3be94be6528fae9cab4f083e85dd75aa9dd25d8df6f353d325861163465badfa22683fdc27",
        delinearized_priv_keys: &[
            "53c4fabe7f845a78d56ae8d482d32cae5e35c588824744b9ae095b0a4e6eac09",
            "821c6db9a9f12f6180d2d45ab421759f0531e1345bba3b0ca686c9e301069c08",
            "a22bb26d3d0494198676230260dd3dc5cdb5457fca6eab5c38bf3d47fff1ab08",
            "a83d01c79751c0bb29387c6d5981c7c3eb1861689d317011c839ec04d888cb0b",
        ],
        delinearized_pub_keys: &[
            "33dc44d3384019a18a4acb951faf37d64870ee890663e0018e71845946626e8c",
            "78b47f19c8b94f5c32fb461bd91c8dcd7025de844fdb777051d0b24de75d51f9",
            "8e5e398f9ab1a96cd27422967d5930a0dd372ddcc3e5c19b2035ca122c59a21e",
            "16e8edc1847db6f52a21c7663ed85b11537c4410b8e5f0ff59ca0188dd7eb633",
        ],
        secrets: &[
            "d671a40fabf8735899862b00395c864ac32abee0aa899ced4d9a22b827311802",
            "d8d568e93fa27da70dae66dde3d7f1b12f2a823e7fa7dffdf0ba3ee65fbd690c",
            "0886a14851375489b3dd95a0d03d4d6c7783759fab0a74d71b2028e99db5230c",
            "2413f3d8415c9afdd1b634ce86b2a6dcaad5cbf2fcb33718d020cf1813ec4705",
        ],
        commitments: &[
            "877c7007fc838154cbacef7d4752d55c44400f0359399b3f59be19422b3eb912",
            "7d1dcbeae1bd00b0ded04ccf76909523535ae2df625711d2461713fe4c113789",
            "4e61ad68f9c163b89dfad6a17f528d5948fb2da1400386d21bab756e630d2fcf",
            "773fafeec83cc008fbf63961d98b56612811bb8037b9f3b192738db3fa6e1d29",
        ],
        agg_commitment: "b57b3ee1fa93230e68def1f56ab9c8e390527618b6f190b592ca40f84a877ad9",
        agg_pub_key: "387d7d274e8c48379afc8ccc91e309bbdd6c259588e3f3999efcc3f137f92e76",
        partial_signatures: &[
            "e4b7de85519f7724fd7d00b70edfe3505a71fe720dc9d34618dd77ab3c96e50f",
            "aa316a69be1fd834fbaa8d76170cb4031adc34ed27ab1640cd44c51aeda55806",
            "51c3c81056912c1445e9b977935233d55aef075187088107c2ce9f33fc00dc07",
            "90d28225e5d752b5e02f60d7ac461ee1d4ee1c713ffc3d1c43657cb5c1031008",
        ],
        agg_signature: "95d7a86b1662aa727108b936a9902be1a32b5822fc78a9aaea5559afe7402a06",
        signature: "b57b3ee1fa93230e68def1f56ab9c8e390527618b6f190b592ca40f84a877ad995d7a86b1662aa727108b936a9902be1a32b5822fc78a9aaea5559afe7402a06",
        message: "",
    },
    StrTestVector {
        priv_keys: &[
            "65edb8c173fdbaf5e106ca53069cde47c2a7627518228d8269a3da35f5fd0001",
            "574e47b97d5918ee12f3872c7843b789c34298ff7cc59ea049586afe63f7c20a",
            "a8177dccd4557044db70f3066960a1df283ba5f0b8ffa997c6411c9119ac160d",
            "21ea275ae38602ef65aac6774db8ed2e6164923b14a7e11c40df874f453b780a",
        ],
        pub_keys: &[
            "3d3f83dc9bacfc90f64b520099e97a81e90ab58a1e10b5c95631c07839c027d7",
            "c5539285ec6a3cca991eb8ce3d0f56311aefa706645db278b6a243fac3ba36ff",
            "d60bcfbce4d869b7de3a6efa2ce12565e77233eb091a34bcee81e6981ddf897b",
            "43b51bc6464d60d95ea36f173f62e23f5405160790aa925694ccfba01d834f18",
        ],
        pub_keys_hash: "2ce18109c27ef0f1112b59e1decbd25b3cfc0347112bc75b053637e11021da8d5e0512e82b64ccd8054da021973f45277e275a0fa6bd89f9bbef8015462c3471",
        delinearized_priv_keys: &[
            "f2bdeb36deadcd5d2f46dd8e72e1bc99a06e305c758a228ef499351220f86c01",
            "ecdd4913aa1bfeb22d22b1c91b3945eb4ce3018bb7f900ba2bee6fefd6b44b03",
            "36b71860698c9efbb302c88bae15824be1a0ceabaa7b0c6212851321be6e5e08",
            "a2949fdc45b8b723c523540964df3d3782139e0dcb65c8145d2556cf5490a202",
        ],
        delinearized_pub_keys: &[
            "48ecd45dcef9a84152cf5aa7ed7f2c884aaafab4a8c7d18b0074169ffc442f1f",
            "39a191c2c6eabb8b0485ea00fe5b7da022a4fa95a8c62c9e51421338f33dea34",
            "9b9e0b154168d124d35eda474592119e6425bceb90be32d0cc9961c9ec9f1597",
            "25c98cb4a563c88c9ac4945e1ba0e83d2ecf6b7624a8f94c9f8a8f3640156079",
        ],
        secrets: &[
            "d8bf664734b0fe7ad84dacd52b0c1aab7b42b1c87de456e666b0473446cee50d",
            "df62a5936edf38fdebf1b77c0e88b67a3c3049c20e9b82042d18e799d9147f00",
            "62f9d569b6a4923a282d6ae5459f4fb9a75a1ac237906198edeb7d71e174b50e",
            "203b3e15c22c24279abaadbb321e2b4d39e1c91183fa20698309f5f1f6154903",
        ],
        commitments: &[
            "b3a8765c83f09db199140b9f877d116b7f70d464004cf7a856f089043ed5623c",
            "efb62e6a34cc50f54118cc4e3a5123cafe440af4245060c7a8bfe0e9de99dac2",
            "1e1d1dfff9ad5e360df85e4ca9496b51fbfc0696a67cfd9e6f6b34a07e3c69e5",
            "74b01bb35dbed39588e3810635b709ecd4547e1f9c4376d5ce3ee5416bbf5e38",
        ],
        agg_commitment: "640d9c1a5e918fed8e28db6c0d3270ff1c45fed558a00215ad42326c77d1b7f8",
        agg_pub_key: "a4f7798ee2e1adf96053b443cc3e2c42d94b15e3fec5ae8637c05b71cc90f7b6",
        partial_signatures: &[
            "9ca3d1ebab436b3309cd37be7a81f3778f1831ae91ee446ae056d1baa8e30b0a",
            "1fcc5bca7d16245d34a6b363586f20f7280eacf42b9ddeeaa664501fd1a09a0f",
            "03e7274c1c8a2a2aeb306a1331a26431c8a80ec84bd2717eb6a4e6b66f4c090d",
            "9800eb815f4df822198970b032871ec364a6951cd47f5e49e0bc0819968e8102",
        ],
        agg_signature: "7caf54ca706b8d2d95f3d69f7926d939e5758187ddddf31c1e1d11aa7f5f3109",
        signature: "640d9c1a5e918fed8e28db6c0d3270ff1c45fed558a00215ad42326c77d1b7f87caf54ca706b8d2d95f3d69f7926d939e5758187ddddf31c1e1d11aa7f5f3109",
        message: "Hello World",
    },
];

#[test]
fn it_can_construct_public_keys() {
    for vector in VECTORS.iter() {
        let test = TestVector::from_str(vector);

        for i in 0..test.priv_keys.len() {
            let public_key = Ed25519PublicKey::from(&test.priv_keys[i]);
            assert_eq!(public_key, test.pub_keys[i]);
        }
    }
}

#[test]
fn it_correctly_calculates_commitments() {
    // Create random 32 bytes.
    let randomness: [u8; RandomSecret::SIZE] = [
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
fn it_correctly_aggregates_commitments() {
    let test = TestVector::from_str(&VECTORS[0]);
    let agg: Commitment = test.commitments.iter().sum();

    assert_eq!(
        agg.to_bytes(),
        hex::decode("5f6591c8d8a304fbdbb259826f8d88210c3aefce858f2432e91ba7224316861b")
            .unwrap()
            .as_slice()
    );
}

#[test]
fn it_can_aggregate_commitments() {
    for (i, vector) in VECTORS.iter().enumerate() {
        let test = TestVector::from_str(vector);

        let aggregated_commitment: Commitment = test.commitments.iter().sum();
        assert_eq!((i, aggregated_commitment), (i, test.agg_commitment));
    }
}

#[test]
fn it_can_aggregate_public_keys() {
    for vector in VECTORS.iter() {
        let test = TestVector::from_str(vector);

        let delinearized_pk_sum: EdwardsPoint = test.delinearized_pub_keys.iter().sum();
        let mut public_key_bytes: [u8; Ed25519PublicKey::SIZE] = [0u8; Ed25519PublicKey::SIZE];
        public_key_bytes.copy_from_slice(delinearized_pk_sum.compress().as_bytes());
        let aggregated_public_key = Ed25519PublicKey::from(public_key_bytes);
        assert_eq!(aggregated_public_key, test.agg_pub_key);
    }
}

#[test]
fn it_can_finalize_signatures() {
    for vector in VECTORS.iter() {
        let test = TestVector::from_str(vector);

        let signature: Ed25519Signature = test.agg_signature.to_signature(&test.agg_commitment);
        assert_eq!(signature, test.signature);
    }
}

#[test]
fn it_can_aggregate_partial_signatures() {
    for vector in VECTORS.iter() {
        let test = TestVector::from_str(vector);

        let partial_signature: PartialSignature = test.partial_signatures.iter().sum();
        assert_eq!(partial_signature, test.agg_signature);
    }
}

#[test]
fn it_can_create_partial_signatures() {
    for vector in VECTORS.iter() {
        let test = TestVector::from_str(vector);

        for i in 0..test.priv_keys.len() {
            let public_keys: Vec<Ed25519PublicKey> = test.pub_keys.to_vec();
            let key_pair = KeyPair::from(test.priv_keys[i].clone());
            let (partial_signature, agg_public_key, agg_commitment) = key_pair.partial_sign(
                &public_keys,
                &test.secrets[i],
                &test.commitments,
                test.message.as_slice(),
            );
            assert_eq!(agg_public_key, test.agg_pub_key);
            assert_eq!(agg_commitment, test.agg_commitment);
            assert_eq!(partial_signature, test.partial_signatures[i]);
        }
    }
}

#[test]
fn it_sign_and_verify_multisigs() {
    for vector in VECTORS.iter() {
        let test = TestVector::from_str(vector);

        let mut signatures: Vec<PartialSignature> = Vec::new();
        let mut aggregated_public_key: Option<Ed25519PublicKey> = None;
        let mut aggregated_commitment: Option<Commitment> = None;

        for i in 0..test.priv_keys.len() {
            let cp = CommitmentPair::new(&test.secrets[i], &test.commitments[i]);
            let key_pair = KeyPair::from(test.priv_keys[i].clone());

            let (partial_signature, agg_public_key, agg_commitment) = key_pair.partial_sign(
                &test.pub_keys,
                cp.random_secret(),
                &test.commitments,
                test.message.as_slice(),
            );

            aggregated_public_key = match aggregated_public_key {
                None => Some(agg_public_key),
                Some(pk) => {
                    assert_eq!(pk, agg_public_key);
                    Some(pk)
                }
            };
            aggregated_commitment = match aggregated_commitment {
                None => Some(agg_commitment),
                Some(com) => {
                    assert_eq!(com, agg_commitment);
                    Some(com)
                }
            };
            signatures.push(partial_signature);
        }

        let partial_signature: PartialSignature = signatures.iter().sum();
        let final_signature: Ed25519Signature =
            partial_signature.to_signature(&aggregated_commitment.unwrap());
        assert!(aggregated_public_key
            .unwrap()
            .verify(&final_signature, test.message.as_slice()));
    }
}

#[test]
fn it_can_hash_public_keys() {
    for vector in VECTORS.iter() {
        let test = TestVector::from_str(vector);

        assert_eq!(hash_public_keys(&test.pub_keys), test.pub_keys_hash);
    }
}

#[test]
fn it_can_delinearize_private_keys() {
    for vector in VECTORS.iter() {
        let test = TestVector::from_str(vector);

        for i in 0..test.priv_keys.len() {
            let key_pair: KeyPair = KeyPair::from(test.priv_keys[i].clone());
            assert_eq!(
                (i, key_pair.delinearize_private_key(&test.pub_keys_hash)),
                (i, test.delinearized_priv_keys[i])
            );
        }
    }
}

#[test]
fn it_can_delinearize_public_keys() {
    for vector in VECTORS.iter() {
        let test = TestVector::from_str(vector);

        for i in 0..test.priv_keys.len() {
            assert_eq!(
                (i, test.pub_keys[i].delinearize(&test.pub_keys_hash)),
                (i, test.delinearized_pub_keys[i])
            );
        }
    }
}

#[test]
fn it_can_construct_commitments() {
    for (j, vector) in VECTORS.iter().enumerate() {
        let test = TestVector::from_str(vector);

        for i in 0..test.priv_keys.len() {
            let commitment: EdwardsPoint =
                &test.secrets[i].0 * ::curve25519_dalek::constants::ED25519_BASEPOINT_TABLE;
            assert_eq!(
                (j, i, commitment.compress().to_bytes()),
                (j, i, test.commitments[i].to_bytes())
            );
        }
    }
}
