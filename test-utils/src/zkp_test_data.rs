use rand::SeedableRng;
use rand_chacha::ChaCha20Rng;

pub fn get_base_seed() -> ChaCha20Rng {
    let seed = [
        1, 0, 52, 0, 0, 0, 0, 0, 1, 0, 10, 0, 22, 32, 0, 0, 2, 0, 55, 49, 0, 11, 0, 0, 3, 0, 0, 0,
        0, 0, 2, 92,
    ];
    ChaCha20Rng::from_seed(seed)
}

// ZK Proof 1
pub const ZKPROOF_SERIALIZED_IN_HEX: &str = "0000008001220fdb8717a9bf71e0b4c3011e1c134c7cba4b82ff310632ec8a920ba66f1b0c945acbfedec653fc1af7cd88b359808e9f6fde2f915dfa2e77283e1f71c52bdc865e2395f7c5a1ff7a9c0b40017727b4f253045a6d74f29e81e97313c3c280d2c0b17c71f303be0556b6785eeaa472fadbe7f1aa2f53b55a25dedc959e5d57511a164bcdfdea4fde6acbc558d77cb56485879b6ed9c0e87bd2a395786246fe93b5ac136aad78838ca124a535b61fdea682e251baf0d08358f9a4704581018a6fc855449d005ac83b77e9a9810c9e26efb1b77b95fb2b2e00f7c03a0fe1bb17c96a0a6d720f97c80c6da1e0667a6857109d5ce2821398ec99f1e9fdd4d3b35d13ceae5a48c5b9abf6020e2bfb9475945decc88334682fb57e426497470047e7764ea16e8076ea6b1043ed2616a5cd4bb189e83016ce0979ebb803eae6a47aa72ae00f183d182ddec0476b5a279627ebfd1f263766785442e1ef33956edd0a2f43d1a832977b844353eb3f131f084429840021549a3bf8422e586c8701a5164ed1da7551724b7f2407e3b02e8238848f70f8e91b72c165de1efd7ed48c9715de02cda9f14edd4dd3523ac016255b232d7c6dfa7f4e1ed61bb8f34e4d886874f98004883b597f6210eab889b2c255680189c73568ec2d2ade6b8d9801";

// ZK Proof 2
pub const ZKPROOF_SERIALIZED_IN_HEX2: &str = "0000010001e2d633811c8fb27765b33facac3da9c6bd7bc067012d483bff3d601535d3ddcdc2e132dccec3d43962ba9de93e659ecd0d37329e0371ff18037b9369cfedd5ba87e00bfb1830208774eb5fa93d66bc494b9a029f7aae9bda0ed2cf45165480024aac8d3daf2caa9c8abdbadac76e98d46b0c745add18cf071ec8e0e5bd84b072ebda441ce75d3ec14c8fa6b3497f913b9d1696d5ced87b6f5b596fdcdcedfcd43a57ccf9264ec50bd0d8872aea19a862affc30f382fa5a7f092c914b50013a0c3a11054ad677682a29920cc0c9c657416eb5da821fcc0908600b22e75c12d1d37b17a347e8444a93c8cce2e4c50604a84d059251bee63c14cf4d7fbb5e7f72fbca80a8d3e77e625fbb7deda212c81a9ed4c6dd80f0bfae31c2490fba00c6d1bbe5267192448bd2ef85904ab37ab6e872eced7bb94eb59c9db94f21d0bbe20fc21b25d8d0b57354402188190d645f967c7a4884abd081b56a4a8a340aa3921b973636b7ffdff444b26de42016f4c16aab9b605360e14475722d288a01213b61acdf772d5eee77601c191a3d0db9414afc1841e8156e643f416fbb70020b15c8dd942c074e6a32ad12cf1d493d78327d4457b8c59cc905cec54bb8c3bf306868fa99a228007ceab9a92bb17adf6ad5c1cdda86348bdeed089732ed80";

pub const ZKP_TEST_BIN_NAME: &str = "nimiq-test-prove";
pub const ZKP_TEST_KEYS_PATH: &str = "../.zkp";

pub fn zkp_test_exe() -> std::path::PathBuf {
    // Cargo puts the integration test binary in target/debug/deps
    let current_exe =
        std::env::current_exe().expect("Failed to get the path of the integration test binary");
    let current_dir = current_exe
        .parent()
        .expect("Failed to get the directory of the integration test binary");

    let test_bin_dir = current_dir
        .parent()
        .expect("Failed to get the binary folder");
    let mut path = test_bin_dir.to_owned();

    path.push(ZKP_TEST_BIN_NAME);
    path.set_extension(std::env::consts::EXE_EXTENSION);

    assert!(
        path.exists(),
        "Run `cargo test --all-features` to build the test prover binary at {path:?}"
    );
    path
}
