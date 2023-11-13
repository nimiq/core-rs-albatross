use nimiq_keys::{
    Address, AddressParseError, KeyPair, PrivateKey, PublicKey, SecureGenerate, Signature,
    WebauthnPublicKey,
};
use nimiq_test_log::test;
use nimiq_test_utils::test_rng::test_rng;

mod multisig;

#[test]
fn verify_created_signature() {
    let key_pair = KeyPair::generate(&mut test_rng(false));
    let data = b"test";
    let signature = key_pair.sign(data);
    let valid = key_pair.public.verify(&signature, data);
    assert!(valid);
}

#[test]
fn verify_webauthn_signature() {
    // All test data was generated in a browser with the Ledger FIDO 2FA applet

    // AuthenticatorData || sha256(ClientDataJSON)
    let data = &[
        73u8, 150, 13, 229, 136, 14, 140, 104, 116, 52, 23, 15, 100, 118, 96, 91, 143, 228, 174,
        185, 162, 134, 50, 199, 153, 92, 243, 186, 131, 29, 151, 99, 1, 101, 1, 154, 106, 138, 153,
        25, 112, 197, 225, 41, 246, 147, 197, 11, 178, 21, 151, 140, 17, 34, 112, 214, 130, 197,
        132, 20, 200, 181, 91, 26, 68, 253, 7, 239, 94,
    ];

    let public_key = WebauthnPublicKey::from_bytes(&[
        2u8, 145, 87, 130, 102, 84, 114, 146, 139, 254, 114, 194, 134, 155, 187, 214, 188, 12, 35,
        147, 121, 213, 161, 80, 234, 94, 43, 25, 178, 5, 213, 54, 89,
    ])
    .unwrap();

    let signature = Signature::from_bytes(&[
        148u8, 2, 16, 13, 80, 112, 18, 235, 56, 73, 56, 148, 250, 186, 193, 159, 178, 162, 217, 86,
        49, 227, 83, 240, 200, 118, 235, 0, 115, 23, 160, 77, 109, 60, 152, 94, 181, 70, 225, 67,
        46, 237, 127, 58, 170, 213, 255, 250, 115, 146, 83, 214, 10, 133, 7, 182, 68, 34, 244, 243,
        111, 11, 52, 213,
    ])
    .unwrap();

    let valid = public_key.verify(&signature, data);
    assert!(valid);
}

#[test]
fn falsify_wrong_signature() {
    let key_pair = KeyPair::generate(&mut test_rng(false));
    let signature = key_pair.sign(b"test");
    let valid = key_pair.public.verify(&signature, b"test2");
    assert!(!valid);
}

#[test]
fn verify_rfc8032_test_vectors() {
    struct TestVector<'a, 'b, 'c, 'd> {
        private: &'a str,
        public: &'b str,
        sig: &'c str,
        msg: &'d str,
    }
    const TEST_CASES: [TestVector; 5] = [
        TestVector {
            private: "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60",
            public: "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a",
            sig: "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b",
            msg: ""
        },
        TestVector {
            private: "4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb",
            public: "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c",
            sig: "92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00",
            msg: "72"
        },
        TestVector {
            private: "c5aa8df43f9f837bedb7442f31dcb7b166d38535076f094b85ce3a2e0b4458f7",
            public: "fc51cd8e6218a1a38da47ed00230f0580816ed13ba3303ac5deb911548908025",
            sig: "6291d657deec24024827e69c3abe01a30ce548a284743a445e3680d7db5ac3ac18ff9b538d16f290ae67f760984dc6594a7c15e9716ed28dc027beceea1ec40a",
            msg: "af82"
        },
        TestVector {
            private: "f5e5767cf153319517630f226876b86c8160cc583bc013744c6bf255f5cc0ee5",
            public: "278117fc144c72340f67d0f2316e8386ceffbf2b2428c9c51fef7c597f1d426e",
            sig: "0aab4c900501b3e24d7cdf4663326a3a87df5e4843b2cbdb67cbf6e460fec350aa5371b1508f9f4528ecea23c436d94b5e8fcd4f681e30a6ac00a9704a188a03",
            msg: "08b8b2b733424243760fe426a4b54908632110a66c2f6591eabd3345e3e4eb98fa6e264bf09efe12ee50f8f54e9f77b1e355f6c50544e23fb1433ddf73be84d879de7c0046dc4996d9e773f4bc9efe5738829adb26c81b37c93a1b270b20329d658675fc6ea534e0810a4432826bf58c941efb65d57a338bbd2e26640f89ffbc1a858efcb8550ee3a5e1998bd177e93a7363c344fe6b199ee5d02e82d522c4feba15452f80288a821a579116ec6dad2b3b310da903401aa62100ab5d1a36553e06203b33890cc9b832f79ef80560ccb9a39ce767967ed628c6ad573cb116dbefefd75499da96bd68a8a97b928a8bbc103b6621fcde2beca1231d206be6cd9ec7aff6f6c94fcd7204ed3455c68c83f4a41da4af2b74ef5c53f1d8ac70bdcb7ed185ce81bd84359d44254d95629e9855a94a7c1958d1f8ada5d0532ed8a5aa3fb2d17ba70eb6248e594e1a2297acbbb39d502f1a8c6eb6f1ce22b3de1a1f40cc24554119a831a9aad6079cad88425de6bde1a9187ebb6092cf67bf2b13fd65f27088d78b7e883c8759d2c4f5c65adb7553878ad575f9fad878e80a0c9ba63bcbcc2732e69485bbc9c90bfbd62481d9089beccf80cfe2df16a2cf65bd92dd597b0707e0917af48bbb75fed413d238f5555a7a569d80c3414a8d0859dc65a46128bab27af87a71314f318c782b23ebfe808b82b0ce26401d2e22f04d83d1255dc51addd3b75a2b1ae0784504df543af8969be3ea7082ff7fc9888c144da2af58429ec96031dbcad3dad9af0dcbaaaf268cb8fcffead94f3c7ca495e056a9b47acdb751fb73e666c6c655ade8297297d07ad1ba5e43f1bca32301651339e22904cc8c42f58c30c04aafdb038dda0847dd988dcda6f3bfd15c4b4c4525004aa06eeff8ca61783aacec57fb3d1f92b0fe2fd1a85f6724517b65e614ad6808d6f6ee34dff7310fdc82aebfd904b01e1dc54b2927094b2db68d6f903b68401adebf5a7e08d78ff4ef5d63653a65040cf9bfd4aca7984a74d37145986780fc0b16ac451649de6188a7dbdf191f64b5fc5e2ab47b57f7f7276cd419c17a3ca8e1b939ae49e488acba6b965610b5480109c8b17b80e1b7b750dfc7598d5d5011fd2dcc5600a32ef5b52a1ecc820e308aa342721aac0943bf6686b64b2579376504ccc493d97e6aed3fb0f9cd71a43dd497f01f17c0e2cb3797aa2a2f256656168e6c496afc5fb93246f6b1116398a346f1a641f3b041e989f7914f90cc2c7fff357876e506b50d334ba77c225bc307ba537152f3f1610e4eafe595f6d9d90d11faa933a15ef1369546868a7f3a45a96768d40fd9d03412c091c6315cf4fde7cb68606937380db2eaaa707b4c4185c32eddcdd306705e4dc1ffc872eeee475a64dfac86aba41c0618983f8741c5ef68d3a101e8a3b8cac60c905c15fc910840b94c00a0b9d0"
        },
        TestVector {
            private: "833fe62409237b9d62ec77587520911e9a759cec1d19755b7da901b96dca3d42",
            public: "ec172b93ad5e563bf4932c70e1245034c35467ef2efd4d64ebf819683467e2bf",
            sig: "dc2a4459e7369633a52b1bf277839a00201009a3efbf3ecb69bea2186c26b58909351fc9ac90b3ecfdfbc7c66431e0303dca179c138ac17ad9bef1177331a704",
            msg: "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
        }
    ];

    for test_case in &TEST_CASES {
        let mut private_key_bytes: [u8; PrivateKey::SIZE] = [0; PrivateKey::SIZE];
        private_key_bytes.clone_from_slice(&::hex::decode(test_case.private).unwrap()[0..]);
        let private_key = PrivateKey::from(&private_key_bytes);

        let mut public_key_bytes: [u8; PublicKey::SIZE] = [0; PublicKey::SIZE];
        public_key_bytes.clone_from_slice(&::hex::decode(test_case.public).unwrap()[0..]);
        let public_key = PublicKey::from(&public_key_bytes);

        let mut sig_key_bytes: [u8; Signature::SIZE] = [0; Signature::SIZE];
        sig_key_bytes.clone_from_slice(&::hex::decode(test_case.sig).unwrap()[0..]);
        let signature = Signature::from(&sig_key_bytes);

        let data = &::hex::decode(test_case.msg).unwrap()[0..];

        let derived_public_key = PublicKey::from(&private_key);

        assert!(derived_public_key == public_key);

        let key_pair = KeyPair {
            private: private_key,
            public: public_key,
        };

        let computed_signature = key_pair.sign(data);

        assert!(computed_signature == signature);
        assert!(public_key.verify(&signature, data));
    }
}

#[test]
fn it_computes_friendly_addresses() {
    let mut addr = Address::from([0u8; Address::SIZE]);
    assert_eq!(
        addr.to_user_friendly_address(),
        "NQ07 0000 0000 0000 0000 0000 0000 0000 0000"
    );

    let mut addr_bytes: [u8; Address::SIZE] = [0; Address::SIZE];
    addr_bytes
        .clone_from_slice(&::hex::decode("e9910f2452419823dc2e5534633210074ae9527f").unwrap()[0..]);
    addr = Address::from(addr_bytes);
    assert_eq!(
        addr.to_user_friendly_address(),
        "NQ97 V68G X92J 86C2 7P1E ALS6 6CGG 0V5E JLKY"
    );

    addr_bytes
        .clone_from_slice(&::hex::decode("2987c28c1ff373ba1e18a9a2efe6dc101ee25ed9").unwrap()[0..]);
    addr = Address::from(addr_bytes);
    assert_eq!(
        addr.to_user_friendly_address(),
        "NQ05 563U 530Y XDRT L7GQ M6HE YRNU 20FE 4PNR"
    );

    let addr2 = Address::from_user_friendly_address("NQ05 563U 530Y XDRT L7GQ M6HE YRNU 20FE 4PNR")
        .unwrap();
    assert_eq!(addr.as_bytes(), addr2.as_bytes());
    assert_eq!(
        addr.to_user_friendly_address(),
        addr2.to_user_friendly_address()
    );
}

#[test]
fn it_parses_friendly_addresses() {
    let addr = Address::from_user_friendly_address("NQ05 563U 530Y XDRT L7GQ M6HE YRNU 20FE 4PNR");
    assert!(addr.is_ok());

    // Not having spaces should be ok
    let addr = Address::from_user_friendly_address("NQ05563U530YXDRTL7GQM6HEYRNU20FE4PNR");
    assert!(addr.is_ok());

    // Not having some spaces should be ok
    let addr = Address::from_user_friendly_address("NQ05 563U 530Y XDRTL7GQ M6HE YRNU 20FE 4PNR");
    assert!(addr.is_ok());

    // Having NQ in lowercase at the beggining should not be ok
    let addr = Address::from_user_friendly_address("nq05 563U 530Y XDRT L7GQ M6HE YRNU 20FE 4PNR");
    assert_eq!(addr, Err(AddressParseError::WrongCountryCode));

    // Wrong Country Code
    let addr = Address::from_user_friendly_address("SQ05 563U 530Y XDRT L7GQ M6HE YRNU 20FE 4PNR");
    assert_eq!(addr, Err(AddressParseError::WrongCountryCode));

    // Wrong Length
    let addr = Address::from_user_friendly_address("SQ05 563U 530Y XDRT L7GQ M6HE YRNU 20FE 4PNRS");
    assert_eq!(addr, Err(AddressParseError::WrongLength));

    // Wrong alphabet (lowercase -> L7gq)
    let addr = Address::from_user_friendly_address("NQ05 563U 530Y XDRT L7gq M6HE YRNU 20FE 4PNR");
    assert_eq!(addr, Err(AddressParseError::UnknownFormat));

    // Wrong checksum
    let addr = Address::from_user_friendly_address("NQ05 563U 530Y XDRT L7IQ M6HE YRNU 20FE 4PNR");
    assert_eq!(addr, Err(AddressParseError::InvalidChecksum));

    // Wrong alphabet (VIDM)
    let addr = Address::from_user_friendly_address("NQ16 GB8S Q5QR MAVN MR3C VIDM 62G6 NL0D ANYX");
    assert_eq!(addr, Err(AddressParseError::UnknownFormat));
}
