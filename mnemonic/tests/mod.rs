use hex;

use nimiq_mnemonic::*;

struct TestVector {
    entropy: &'static str,
    mnemonic: &'static str,
    seed: Option<&'static str>,
}

impl TestVector {
    fn get_entropy(&self) -> Entropy {
        Entropy::from(self.entropy)
    }

    fn get_mnemonic(&self) -> Mnemonic {
        Mnemonic::from(self.mnemonic)
    }

    fn entropy_len(&self) -> usize {
        self.entropy.len() / 2
    }

    fn get_seed(&self) -> Vec<u8> {
        hex::decode(self.seed.unwrap()).unwrap()
    }
}

#[test]
fn it_correctly_computes_mnemonics() {
    // Test vectors from https://github.com/trezor/python-mnemonic/blob/master/vectors.json
    const TEST_CASES: [TestVector; 24] = [
        TestVector {
            entropy: "00000000000000000000000000000000",
            mnemonic: "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
            seed: Some("c55257c360c07c72029aebc1b53c05ed0362ada38ead3e3e9efa3708e53495531f09a6987599d18264c1e1c92f2cf141630c7a3c4ab7c81b2f001698e7463b04"),
        },
        TestVector {
            entropy: "7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f",
            mnemonic: "legal winner thank year wave sausage worth useful legal winner thank yellow",
            seed: Some("2e8905819b8723fe2c1d161860e5ee1830318dbf49a83bd451cfb8440c28bd6fa457fe1296106559a3c80937a1c1069be3a3a5bd381ee6260e8d9739fce1f607"),
        },
        TestVector {
            entropy: "80808080808080808080808080808080",
            mnemonic: "letter advice cage absurd amount doctor acoustic avoid letter advice cage above",
            seed: Some("d71de856f81a8acc65e6fc851a38d4d7ec216fd0796d0a6827a3ad6ed5511a30fa280f12eb2e47ed2ac03b5c462a0358d18d69fe4f985ec81778c1b370b652a8"),
        },
        TestVector {
            entropy: "ffffffffffffffffffffffffffffffff",
            mnemonic: "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrong",
            seed: Some("ac27495480225222079d7be181583751e86f571027b0497b5b5d11218e0a8a13332572917f0f8e5a589620c6f15b11c61dee327651a14c34e18231052e48c069"),
        },
        TestVector {
            entropy: "000000000000000000000000000000000000000000000000",
            mnemonic: "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon agent",
            seed: Some("035895f2f481b1b0f01fcf8c289c794660b289981a78f8106447707fdd9666ca06da5a9a565181599b79f53b844d8a71dd9f439c52a3d7b3e8a79c906ac845fa"),
        },
        TestVector {
            entropy: "7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f",
            mnemonic: "legal winner thank year wave sausage worth useful legal winner thank year wave sausage worth useful legal will",
            seed: Some("f2b94508732bcbacbcc020faefecfc89feafa6649a5491b8c952cede496c214a0c7b3c392d168748f2d4a612bada0753b52a1c7ac53c1e93abd5c6320b9e95dd"),
        },
        TestVector {
            entropy: "808080808080808080808080808080808080808080808080",
            mnemonic: "letter advice cage absurd amount doctor acoustic avoid letter advice cage absurd amount doctor acoustic avoid letter always",
            seed: Some("107d7c02a5aa6f38c58083ff74f04c607c2d2c0ecc55501dadd72d025b751bc27fe913ffb796f841c49b1d33b610cf0e91d3aa239027f5e99fe4ce9e5088cd65"),
        },
        TestVector {
            entropy: "ffffffffffffffffffffffffffffffffffffffffffffffff",
            mnemonic: "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo when",
            seed: Some("0cd6e5d827bb62eb8fc1e262254223817fd068a74b5b449cc2f667c3f1f985a76379b43348d952e2265b4cd129090758b3e3c2c49103b5051aac2eaeb890a528"),
        },
        TestVector {
            entropy: "0000000000000000000000000000000000000000000000000000000000000000",
            mnemonic: "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art",
            seed: Some("bda85446c68413707090a52022edd26a1c9462295029f2e60cd7c4f2bbd3097170af7a4d73245cafa9c3cca8d561a7c3de6f5d4a10be8ed2a5e608d68f92fcc8"),
        },
        TestVector {
            entropy: "7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f",
            mnemonic: "legal winner thank year wave sausage worth useful legal winner thank year wave sausage worth useful legal winner thank year wave sausage worth title",
            seed: Some("bc09fca1804f7e69da93c2f2028eb238c227f2e9dda30cd63699232578480a4021b146ad717fbb7e451ce9eb835f43620bf5c514db0f8add49f5d121449d3e87"),
        },
        TestVector {
            entropy: "8080808080808080808080808080808080808080808080808080808080808080",
            mnemonic: "letter advice cage absurd amount doctor acoustic avoid letter advice cage absurd amount doctor acoustic avoid letter advice cage absurd amount doctor acoustic bless",
            seed: Some("c0c519bd0e91a2ed54357d9d1ebef6f5af218a153624cf4f2da911a0ed8f7a09e2ef61af0aca007096df430022f7a2b6fb91661a9589097069720d015e4e982f"),
        },
        TestVector {
            entropy: "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            mnemonic: "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo vote",
            seed: Some("dd48c104698c30cfe2b6142103248622fb7bb0ff692eebb00089b32d22484e1613912f0a5b694407be899ffd31ed3992c456cdf60f5d4564b8ba3f05a69890ad"),
        },
        TestVector {
            entropy: "9e885d952ad362caeb4efe34a8e91bd2",
            mnemonic: "ozone drill grab fiber curtain grace pudding thank cruise elder eight picnic",
            seed: Some("274ddc525802f7c828d8ef7ddbcdc5304e87ac3535913611fbbfa986d0c9e5476c91689f9c8a54fd55bd38606aa6a8595ad213d4c9c9f9aca3fb217069a41028"),
        },
        TestVector {
            entropy: "6610b25967cdcca9d59875f5cb50b0ea75433311869e930b",
            mnemonic: "gravity machine north sort system female filter attitude volume fold club stay feature office ecology stable narrow fog",
            seed: Some("628c3827a8823298ee685db84f55caa34b5cc195a778e52d45f59bcf75aba68e4d7590e101dc414bc1bbd5737666fbbef35d1f1903953b66624f910feef245ac"),
        },
        TestVector {
            entropy: "68a79eaca2324873eacc50cb9c6eca8cc68ea5d936f98787c60c7ebc74e6ce7c",
            mnemonic: "hamster diagram private dutch cause delay private meat slide toddler razor book happy fancy gospel tennis maple dilemma loan word shrug inflict delay length",
            seed: Some("64c87cde7e12ecf6704ab95bb1408bef047c22db4cc7491c4271d170a1b213d20b385bc1588d9c7b38f1b39d415665b8a9030c9ec653d75e65f847d8fc1fc440"),
        },
        TestVector {
            entropy: "c0ba5a8e914111210f2bd131f3d5e08d",
            mnemonic: "scheme spot photo card baby mountain device kick cradle pact join borrow",
            seed: Some("ea725895aaae8d4c1cf682c1bfd2d358d52ed9f0f0591131b559e2724bb234fca05aa9c02c57407e04ee9dc3b454aa63fbff483a8b11de949624b9f1831a9612"),
        },
        TestVector {
            entropy: "6d9be1ee6ebd27a258115aad99b7317b9c8d28b6d76431c3",
            mnemonic: "horn tenant knee talent sponsor spell gate clip pulse soap slush warm silver nephew swap uncle crack brave",
            seed: Some("fd579828af3da1d32544ce4db5c73d53fc8acc4ddb1e3b251a31179cdb71e853c56d2fcb11aed39898ce6c34b10b5382772db8796e52837b54468aeb312cfc3d"),
        },
        TestVector {
            entropy: "9f6a2878b2520799a44ef18bc7df394e7061a224d2c33cd015b157d746869863",
            mnemonic: "panda eyebrow bullet gorilla call smoke muffin taste mesh discover soft ostrich alcohol speed nation flash devote level hobby quick inner drive ghost inside",
            seed: Some("72be8e052fc4919d2adf28d5306b5474b0069df35b02303de8c1729c9538dbb6fc2d731d5f832193cd9fb6aeecbc469594a70e3dd50811b5067f3b88b28c3e8d"),
        },
        TestVector {
            entropy: "23db8160a31d3e0dca3688ed941adbf3",
            mnemonic: "cat swing flag economy stadium alone churn speed unique patch report train",
            seed: Some("deb5f45449e615feff5640f2e49f933ff51895de3b4381832b3139941c57b59205a42480c52175b6efcffaa58a2503887c1e8b363a707256bdd2b587b46541f5"),
        },
        TestVector {
            entropy: "8197a4a47f0425faeaa69deebc05ca29c0a5b5cc76ceacc0",
            mnemonic: "light rule cinnamon wrap drastic word pride squirrel upgrade then income fatal apart sustain crack supply proud access",
            seed: Some("4cbdff1ca2db800fd61cae72a57475fdc6bab03e441fd63f96dabd1f183ef5b782925f00105f318309a7e9c3ea6967c7801e46c8a58082674c860a37b93eda02"),
        },
        TestVector {
            entropy: "066dca1a2bb7e8a1db2832148ce9933eea0f3ac9548d793112d9a95c9407efad",
            mnemonic: "all hour make first leader extend hole alien behind guard gospel lava path output census museum junior mass reopen famous sing advance salt reform",
            seed: Some("26e975ec644423f4a4c4f4215ef09b4bd7ef924e85d1d17c4cf3f136c2863cf6df0a475045652c57eb5fb41513ca2a2d67722b77e954b4b3fc11f7590449191d"),
        },
        TestVector {
            entropy: "f30f8c1da665478f49b001d94c5fc452",
            mnemonic: "vessel ladder alter error federal sibling chat ability sun glass valve picture",
            seed: Some("2aaa9242daafcee6aa9d7269f17d4efe271e1b9a529178d7dc139cd18747090bf9d60295d0ce74309a78852a9caadf0af48aae1c6253839624076224374bc63f"),
        },
        TestVector {
            entropy: "c10ec20dc3cd9f652c7fac2f1230f7a3c828389a14392f05",
            mnemonic: "scissors invite lock maple supreme raw rapid void congress muscle digital elegant little brisk hair mango congress clump",
            seed: Some("7b4a10be9d98e6cba265566db7f136718e1398c71cb581e1b2f464cac1ceedf4f3e274dc270003c670ad8d02c4558b2f8e39edea2775c9e232c7cb798b069e88"),
        },
        TestVector {
            entropy: "f585c11aec520db57dd353c69554b21a89b20fb0650966fa0a9d6f74fd989d8f",
            mnemonic: "void come effort suffer camp survey warrior heavy shoot primary clutch crush open amazing screen patrol group space point ten exist slush involve unfold",
            seed: Some("01f5bced59dec48e362f2c45b5de68b9fd6c92c6634f44d6d40aab69056506f0e35524a518034ddc1192e1dacd32c1ed3eaa3c3b131c88ed8e7e54c49a5d0998"),
        }
    ];

    for (i, vector) in TEST_CASES
        .iter()
        .filter(|case| case.entropy_len() == 32)
        .enumerate()
    {
        // We only support 32 byte entropy.
        let entropy = vector.get_entropy();
        let mnemonic = vector.get_mnemonic();

        assert_eq!(
            entropy.to_mnemonic(WORDLIST_EN),
            mnemonic,
            "Invalid entropy.to_mnemonic in test case {}",
            i
        );
        let computed_entropy = mnemonic.to_entropy(WORDLIST_EN);
        assert!(
            computed_entropy.is_some(),
            "mnemonic.to_entropy yields None in test case {}",
            i
        );
        assert_eq!(
            mnemonic.to_entropy(WORDLIST_EN).unwrap(),
            entropy,
            "Invalid mnemonic.to_entropy in test case {}",
            i
        );
        if vector.seed.is_some() {
            assert_eq!(mnemonic.to_seed(Some("TREZOR")).unwrap(), vector.get_seed());
        }
    }
}

#[test]
fn it_correctly_computes_mnemonic_type() {
    let bip39 = Mnemonic::from("void come effort suffer camp survey warrior heavy shoot primary clutch crush open amazing screen patrol group space point ten exist slush involve unfold");
    let legacy = Mnemonic::from("refuse walk suggest raven cheese gate eye divert base slot fossil lock oven fuel thank need unit oak image spike vehicle grace citizen expose");
    let both = Mnemonic::from("enforce valid stock pool lonely label awesome olive torch pledge enhance deny you rude inspire ill letter pulse pact light timber enable elbow guide");
    let invalid_checksum = Mnemonic::from("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art abandon abandon abandon abandon abandon abandon art");
    let invalid_words = Mnemonic::from("void come effort suffer camp survey nimiq heavy shoot primary clutch crush open amazing screen patrol group space point ten exist slush involve unfold");
    assert_eq!(bip39.get_type(WORDLIST_EN), MnemonicType::BIP39);
    assert_eq!(legacy.get_type(WORDLIST_EN), MnemonicType::LEGACY);
    assert_eq!(both.get_type(WORDLIST_EN), MnemonicType::UNKNOWN);
    assert_eq!(
        invalid_checksum.get_type(WORDLIST_EN),
        MnemonicType::INVALID
    );
    assert_eq!(invalid_words.get_type(WORDLIST_EN), MnemonicType::INVALID);
}

#[test]
fn it_correctly_detects_collisions() {
    let entropy = Mnemonic::from("enforce valid stock pool lonely label awesome olive torch pledge enhance deny you rude inspire ill letter pulse pact light timber enable elbow guide").to_entropy(WORDLIST_EN).unwrap();
    assert!(entropy.is_colliding_checksum());
}

#[test]
#[allow(deprecated)]
fn it_correctly_computes_legacy_mnemonics() {
    let test_case = TestVector {
        entropy: "a9cb4c81a0e6818d75f0c3efc512c41e1a34fd011551f916ea73f60e3ec5e4df",
        mnemonic: "prefer foil call dove gym shop style blur used chuckle rain destroy person leader affair pretty weekend resemble ostrich ugly token glass nature visa",
        seed: None,
    };
    let entropy = test_case.get_entropy();
    let mnemonic = test_case.get_mnemonic();

    assert_eq!(entropy.to_legacy_mnemonic(WORDLIST_EN), mnemonic);
    let computed_entropy = mnemonic.to_entropy_legacy(WORDLIST_EN);
    assert!(computed_entropy.is_some());
    assert_eq!(computed_entropy.unwrap(), entropy);
}
