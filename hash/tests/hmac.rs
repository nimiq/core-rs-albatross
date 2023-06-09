use hex::FromHex;
use nimiq_hash::{hmac::*, sha512::Sha512Hash};
use nimiq_test_log::test;

struct TestVector {
    key: &'static str,
    data: &'static str,
    hash: &'static str,
}

impl TestVector {
    fn get_key(&self) -> Vec<u8> {
        Vec::from_hex(self.key).unwrap()
    }

    fn get_data(&self) -> Vec<u8> {
        Vec::from_hex(self.data).unwrap()
    }

    fn get_hash(&self) -> Sha512Hash {
        Sha512Hash::from(self.hash)
    }
}

#[test]
fn it_correctly_computes_hmac_sha512() {
    // Test vectors from https://tools.ietf.org/html/rfc4231
    const TEST_CASES: [TestVector; 7] = [
        TestVector {
            key: "0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b",
            data: "4869205468657265",
            hash: "87aa7cdea5ef619d4ff0b4241a1d6cb02379f4e2ce4ec2787ad0b30545e17cdedaa833b7d6b8a702038b274eaea3f4e4be9d914eeb61f1702e696c203a126854"
        },
        TestVector {
            key: "4a656665",
            data: "7768617420646f2079612077616e7420666f72206e6f7468696e673f",
            hash: "164b7a7bfcf819e2e395fbe73b56e0a387bd64222e831fd610270cd7ea2505549758bf75c05a994a6d034f65f8f0e6fdcaeab1a34d4a6b4b636e070a38bce737"
        },
        TestVector {
            key: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            data: "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
            hash: "fa73b0089d56a284efb0f0756c890be9b1b5dbdd8ee81a3655f83e33b2279d39bf3e848279a722c806b485a47e67c807b946a337bee8942674278859e13292fb"
        },
        TestVector {
            key: "0102030405060708090a0b0c0d0e0f10111213141516171819",
            data: "cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd",
            hash: "b0ba465637458c6990e5a8c5f61d4af7e576d97ff94b872de76f8050361ee3dba91ca5c11aa25eb4d679275cc5788063a5f19741120c4f2de2adebeb10a298dd"
        },
        TestVector {
            key: "0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c",
            data: "546573742057697468205472756e636174696f6e",
            hash: "415fad6271580a531d4179bc891d87a6"
        },
        TestVector {
            key: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            data: "54657374205573696e67204c6172676572205468616e20426c6f636b2d53697a65204b6579202d2048617368204b6579204669727374",
            hash: "80b24263c7c1a3ebb71493c1dd7be8b49b46d1f41b4aeec1121b013783f8f3526b56d037e05f2598bd0fd2215d6a1e5295e64f73f63f0aec8b915a985d786598"
        },
        TestVector {
            key: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            data: "5468697320697320612074657374207573696e672061206c6172676572207468616e20626c6f636b2d73697a65206b657920616e642061206c6172676572207468616e20626c6f636b2d73697a6520646174612e20546865206b6579206e6565647320746f20626520686173686564206265666f7265206265696e6720757365642062792074686520484d414320616c676f726974686d2e",
            hash: "e37b6a775dc87dbaa4dfa9f96e5e3ffddebd71f8867289865df5a32d20cdc944b6022cac3c4982b10d5eeb55c3e4de15134676fb6de0446065c97440fa8c6a58"
        }
    ];

    for (i, vector) in TEST_CASES.iter().enumerate() {
        let hash = compute_hmac_sha512(&vector.get_key()[..], &vector.get_data()[..]);
        if i == 4 {
            // Simulate truncated output.
            let mut hash = hash.to_string();
            hash.truncate(32);
            assert_eq!(hash, vector.hash, "Invalid hmac sha512 in test case {}", i);
        } else {
            assert_eq!(
                hash,
                vector.get_hash(),
                "Invalid hmac sha512 in test case {}",
                i
            );
        }
    }
}
