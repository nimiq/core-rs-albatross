use rand::rngs::OsRng;
pub use rand::{CryptoRng, Rng};

pub type SecureRng = OsRng;

pub trait SecureGenerate: Sized {
    fn generate<R: Rng + CryptoRng>(rng: &mut R) -> Self;

    #[inline]
    fn generate_default_csprng() -> Self {
        SecureGenerate::generate(&mut SecureRng::default())
    }
}
