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
pub const ZKPROOF_SERIALIZED_IN_HEX: &str = "0000008001da9eb069a214d1d649fcd8d1b4565080835959fa633cc0a4fafee8234c830015a8a18baad150526f90bec3ab917afc08741eeaf8aa3608fa5b5325be8eb4202b969413e99515307df43e2515ad8b996597857a89dd37e13d784ac674e6338101e9a76ba5f00b853c352560ebf6f5e001b92130dd0325638f8ea90ee85b928287c6ac980ebf108a84d53851dabc4e5796b6f9dff585562a6f80635a5cefd4362271ac105ec6c4f9bf51b4dd1bbcff66b9fe8958e19cf0a7868eadf8483201d141a243e0f08a8f9a8b1bd30a03f20ceeb8f070de1a815697ad288f7a128c386cc97a6117596e8ec69f0f47a6c956b5fb5c5d6acd4fe93e4a95fed9191a617f5980ff5d57ddc189c0d8189f261941b5f368e5660cc000f1d39d53a9bb1401d11870b3a38b0ee68ddcfb677c20f2321b2967cb2e57e31f4c059ffc6c3a4f95028f677b1dd8534f2c3968c0fee83c2c91964ee074a062d4f7cf6319ae2408528a3551b86541b1895f2d4680914424da8575d0667ee1af9d1df896d463a980b1ccaca40544ae38b5e4ca0e2f8300f71b50c110107c127bd3dcfd77c8b7778a67a6140c2b936c4351f7c391951d69f0d162d8a2b7a9fb333d48a78204b2bf3047a9c3ca0a5973d2ebb6e893354aef6130941b21c4b823c964feab85744a80";

/// ZK Proof 2
pub const ZKPROOF_SERIALIZED_IN_HEX2: &str = "00000100012e44ca4f50a844a9eaff5064762709be666732252a116e32b7598ab0b8cb646ab18b9c6b25f4afbeb38a9073c141a65937176cf0744cf350095ab7da3fb48d68f733e2a90fd6d8da844694f96f3a9c7a160c99e99497eee9a4ce6d1692bb00d253d8496077c7ef1a6028d831129ec6ca5aceb65caca4b69278826f0318e5e8993a21975fc61166ca7822f43e1fbaf598aa1eac4f7a8605a653b60a812e6655b4f501828028140b2875e06a8e7cc0c95d2dde59f5e0a1fe0764a236a8810104a33bfe80973361f8de43db31860c4bf19bae562d506f7ba40082f796bd830b2dfbac096cac748e4a55b8bfa66e152f796429ffaa252d1a1357c0c9e2e4635a9b3341469d9f0f4e6dc0a497201965dd575957752340965c0c0662db2ec40179ecaa314ce063dff106ffa79378fa85ec22df8f3596ac57fe504bfdec65c3200ad1ac20ea45b1f0b6a6fc1ceb28080a2e97a2b92abfb904dc27f90443955ea59646a18f9f60d8c75219f536f4044ff98b7e5892c41304aef03a5d9efb50816856cb9d5bae93198813a2ee7b1263506021c0038cf9a0501cd3c897261b8b1160030dbbdd23a903bf3c3e76ab0e1321d830df9e073ece00209b007dfb905df2969eae17ca5ce835c92ed523fa9523806d758486987cb9d477bd9430463d01";

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
