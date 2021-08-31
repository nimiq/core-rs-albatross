use rand_core::OsRng;
pub use rand_core::{CryptoRng, RngCore};

pub type SecureRng = OsRng;

pub trait SecureGenerate: Sized {
    fn generate<R: RngCore + CryptoRng>(rng: &mut R) -> Self;

    #[inline]
    fn generate_default_csprng() -> Self {
        SecureGenerate::generate(&mut SecureRng::default())
    }
}
