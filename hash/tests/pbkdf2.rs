use hex::FromHex;

use nimiq_hash::pbkdf2::*;

struct TestVector {
    password: &'static str,
    salt: &'static str,
    iterations: usize,
    derived_key_length: usize,
    derived_key: &'static str,
}

impl TestVector {
    fn get_password(&self) -> Vec<u8> {
        self.password.as_bytes().to_vec()
    }

    fn get_salt(&self) -> Vec<u8> {
        self.salt.as_bytes().to_vec()
    }

    fn get_derived_key(&self) -> Vec<u8> {
        Vec::from_hex(self.derived_key).unwrap()
    }
}

#[test]
fn it_correctly_computes_hmac_sha512() {
    // Test vectors from https://tools.ietf.org/html/rfc4231
    const TEST_CASES: [TestVector; 5] = [
        TestVector {
            password: "password",
            salt: "salt",
            iterations: 1,
            derived_key_length: 64,
            derived_key: "867f70cf1ade02cff3752599a3a53dc4af34c7a669815ae5d513554e1c8cf252c02d470a285a0501bad999bfe943c08f050235d7d68b1da55e63f73b60a57fce",
        },
        TestVector {
            password: "password",
            salt: "salt",
            iterations: 2,
            derived_key_length: 64,
            derived_key: "e1d9c16aa681708a45f5c7c4e215ceb66e011a2e9f0040713f18aefdb866d53cf76cab2868a39b9f7840edce4fef5a82be67335c77a6068e04112754f27ccf4e",
        },
        TestVector {
            password: "password",
            salt: "salt",
            iterations: 2,
            derived_key_length: 32,
            derived_key: "e1d9c16aa681708a45f5c7c4e215ceb66e011a2e9f0040713f18aefdb866d53c",
        },
        TestVector {
            password: "password",
            salt: "salt",
            iterations: 4096,
            derived_key_length: 64,
            derived_key: "d197b1b33db0143e018b12f3d1d1479e6cdebdcc97c5c0f87f6902e072f457b5143f30602641b3d55cd335988cb36b84376060ecd532e039b742a239434af2d5",
        },
        TestVector {
            password: "passwordPASSWORDpassword",
            salt: "saltSALTsaltSALTsaltSALTsaltSALTsalt",
            iterations: 4096,
            derived_key_length: 64,
            derived_key: "8c0511f4c6e597c6ac6315d8f0362e225f3c501495ba23b868c005174dc4ee71115b59f9e60cd9532fa33e0f75aefe30225c583a186cd82bd4daea9724a3d3b8",
        }
    ];

    for (i, vector) in TEST_CASES.iter().enumerate() {
        let derived_key = compute_pbkdf2_sha512(
            &vector.get_password()[..],
            &vector.get_salt()[..],
            vector.iterations,
            vector.derived_key_length,
        );
        assert_eq!(
            derived_key.unwrap(),
            vector.get_derived_key(),
            "Invalid pbkdf2 in test case {}",
            i
        );
    }
}
