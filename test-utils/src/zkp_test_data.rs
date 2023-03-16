use rand::SeedableRng;
use rand_chacha::ChaCha20Rng;

pub fn get_base_seed() -> ChaCha20Rng {
    let seed = [
        1, 0, 52, 0, 0, 0, 0, 0, 1, 0, 10, 0, 22, 32, 0, 0, 2, 0, 55, 49, 0, 11, 0, 0, 3, 0, 0, 0,
        0, 0, 2, 92,
    ];
    ChaCha20Rng::from_seed(seed)
}

/// ZK Proof 1
pub const ZKPROOF_SERIALIZED_IN_HEX: &str = "0000008001071818ad89f10355f3624da3191780484be2789a1dcc47946e6c30634889b5fb08f7a197c9e00c448dcb980fcc5c5ee27c1c9f566d535e3ccc51d6d3fba80ff46569d88c8bc40991e4c796319ba9fa547a99b043caa296e53bcdd26bccdd802bcc28e2ef8f85e9f3daa6cb11e98ad8830cc17d4436a029adcbc83718a5a26cae71a72a2d9aec2c678ca0758e382c93894f2530f442e29f362329648f10609dee8b98e91fe2450b0781270f3a5e4bb71dfdd2fce3f8810bdd178e3d1c7d013b52370ea7029a7822d0e67b3fd0ed72ac299e69a7a5b601aa8511268e17bffd3a1619a57dee35e9f1c416a4aab55d1ff539ce5a401318c295501329d4a06d673c556ad91671063ba0b90978db5d1922bb1593edc3639c38f9e4a0dde23600a6ab073a3259d5a5728a9c7f5c4e8572137809f63256e6bd6cbed7f2e1b1a76658c04f54b247d71ca8d98c68ecb7492cd44527d6a67385cb9dca90eb16ca663b129399e5a3b022d39b8be32ad6a8f033ae51cbd4945cb5e2d3435cb9772b003d758dc34adc6ec361ba3a9593415a07e54253ba30a6dba5aff9c1b7582e74fffb29ed7d84599a11c27cb27d1b47686fa238b872ab92077201dddaf36e1294664515221b6741a202cf4fdcace0e68ef727f526c846142719dbd9652637a701";

// ZK Proof 2
pub const ZKPROOF_SERIALIZED_IN_HEX2: &str = "0000010001e2d633811c8fb27765b33facac3da9c6bd7bc067012d483bff3d601535d3ddcdc2e132dccec3d43962ba9de93e659ecd0d37329e0371ff18037b9369cfedd5ba87e00bfb1830208774eb5fa93d66bc494b9a029f7aae9bda0ed2cf45165480024aac8d3daf2caa9c8abdbadac76e98d46b0c745add18cf071ec8e0e5bd84b072ebda441ce75d3ec14c8fa6b3497f913b9d1696d5ced87b6f5b596fdcdcedfcd43a57ccf9264ec50bd0d8872aea19a862affc30f382fa5a7f092c914b50013a0c3a11054ad677682a29920cc0c9c657416eb5da821fcc0908600b22e75c12d1d37b17a347e8444a93c8cce2e4c50604a84d059251bee63c14cf4d7fbb5e7f72fbca80a8d3e77e625fbb7deda212c81a9ed4c6dd80f0bfae31c2490fba00c6d1bbe5267192448bd2ef85904ab37ab6e872eced7bb94eb59c9db94f21d0bbe20fc21b25d8d0b57354402188190d645f967c7a4884abd081b56a4a8a340aa3921b973636b7ffdff444b26de42016f4c16aab9b605360e14475722d288a01213b61acdf772d5eee77601c191a3d0db9414afc1841e8156e643f416fbb70020b15c8dd942c074e6a32ad12cf1d493d78327d4457b8c59cc905cec54bb8c3bf306868fa99a228007ceab9a92bb17adf6ad5c1cdda86348bdeed089732ed80";

pub const ZKP_TEST_BIN_NAME: &str = "nimiq-test-prove";
/// The path to the zkp directory for tests relative to the test binaries.
/// This should be used while running unit tests.
pub const ZKP_TEST_KEYS_PATH: &str = "../.zkp_tests";
/// The path to the zkp directory for tests relative to the project root.
/// This should be used while running test related binaries.
/// We have copies of this constant in several places that need to be updated when updating this.
pub const DEFAULT_TEST_KEYS_PATH: &str = ".zkp_tests";

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
