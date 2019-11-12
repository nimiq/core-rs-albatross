pub use rand::{CryptoRng, Rng};
use rand::rngs::OsRng;

pub type SecureRng = OsRng;

pub trait SecureGenerate: Sized {
    #[inline]
    fn generate<R: Rng + CryptoRng>(rng: &mut R) -> Self;

    #[inline]
    fn generate_default_csprng() -> Self {
        SecureGenerate::generate(&mut SecureRng::default())
    }
}