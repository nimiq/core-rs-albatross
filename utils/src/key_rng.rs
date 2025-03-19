use rand::rngs::OsRng;
pub use rand::{CryptoRng, RngCore};
use rand_core::UnwrapErr;

pub type SecureRng = UnwrapErr<OsRng>;

pub trait SecureGenerate: Sized {
    fn generate<R: RngCore + CryptoRng>(rng: &mut R) -> Self;

    #[inline]
    fn generate_default_csprng() -> Self {
        SecureGenerate::generate(&mut SecureRng::default())
    }
}
