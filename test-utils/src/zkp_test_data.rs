use rand::SeedableRng;
use rand_chacha::ChaCha20Rng;

pub fn get_base_seed() -> ChaCha20Rng {
    let seed = [
        1, 0, 52, 0, 0, 0, 0, 0, 1, 0, 10, 0, 22, 32, 0, 0, 2, 0, 55, 49, 0, 11, 0, 0, 3, 0, 0, 0,
        0, 0, 2, 92,
    ];
    ChaCha20Rng::from_seed(seed)
}

//ZK Proof 1
pub const ZKPROOF_SERIALIZED_IN_HEX :&str = "0000008001b3705307fd8274407743a40610fc0f9728565340f5f557ea6d398484bd482809bcbd82d5c13b193e5bb9805b6c6a298b9bc37d9ac0d11e56a33a851edf2c62e5168c9fbbc08dac0ab38abf2da53f4dc05692a0942e68618ae59c0750333e000eb6cd182553bbcb44ae1684edb6f07b3fb5f5f425b091b268effc89112e719bb4537e0aa9eeca7c65fac483cc42672e59e0909f7d78c92fb2c83878ae23ec70abb406a4e8be064c583f7229ef558fbab3706f428596b712b256c660c1d700a2b51b182d3fb97fa1911059b90a9aa4b0c781054f8fbf02076cc20af59ab3cf3b116e9b36040a8d394423270aba501953645b5ad81e3afa96f33cdbeacb18a044b432086b70ca171041372a3fc2d513ff7bc92f76d06ad307b103512804006518fe3bc0c5bd1097f4b5af61488724aa72e83dd6bba0864b6ded3a0672b122fe1ec4b1dcfb4cbca8b70e32dd47e0748da59985c55eff87d9cc67c87aa2059e5596e80460a4d4082b29563f47b64c17d0c4c3b66413d46962355f0a5a95000897ace898d31bda7e55c5204b2502ed9eae9769b6e5f3d938d7b02aaea08eb740f32d654e2001d5cb2e57f3d8a6485a7f3a0143f226faf722fb3abdb00d9c0bf908a435c9aed22b5186eb4c15b9c08fba690ff94bd6283d027f3ca2376381";

//ZK Proof 2
pub const ZKPROOF_SERIALIZED_IN_HEX2 :&str = "0000010001cf6971bd41c16551758cdb2eb31a394729b282b1b33065531c3c9ac59ef485d2b3621ebfbf146fa23729f634acd684cf68434d358a6534935cdf55282a4c2ab3f750d9ca7e19e132ed633bbc0c23cc270cb9802fa287377ea59c1c78abe1802c1bf9d83338a27d1050d17bc5cafc6381bd1885fab2c56492c92d0810c0d79d663688aa195fb502bece6ce994cb30a82af334b95c305855844a9c90381231b3be45621c0cd4938532b2e27d353f57dd12700fdece42b606f69ef04f617500d1b2da98c2a73470f8ab1e024ef34eb097debc11c2bc9ef9d949276f365c0aaddc34c36327ffaf1d98512da2e28fe37febb4a06331721a068b42ddc65f318037c0457bebbe2d9aa506f1e1028ec65d5e870de999e9e1ce403d041c71558101b2ad4cbd20fbe29aada17b67af1c4d79a3fe3dae0c30149c43c5d76f250195d5abf086f722d6b4f13c39385591a83b1427fc744d2aa0dcb3d7ac6385f0ca6fce92615e48256b5cf595fe6e37f804d365ef04b4b0c2e7a04138904aeae8eb00142eac4a42577673aa4ea4b8b3397e67433190f9e672b8d24bee6346ad3ed04f23f32544d427c9e77d6a613a23ea3bdc459506c5f9d5e9c0195174680c8b453d042ae3a65b56239f224e7129ea09aa76bc0a7dec3e3ec1f6bad1f0acc6c880";

pub const ZKP_TEST_BIN_NAME: &str = "nimiq-prove";

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

    assert!(path.exists());
    path
}
