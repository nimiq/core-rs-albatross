use rand::rngs::SysRng;
pub use rand::{CryptoRng, Rng};
use rand_core::UnwrapErr;

pub type SecureRng = UnwrapErr<SysRng>;

pub trait SecureGenerate: Sized {
    fn generate<R: Rng + CryptoRng>(rng: &mut R) -> Self;

    #[inline]
    fn generate_default_csprng() -> Self {
        SecureGenerate::generate(&mut SecureRng::default())
    }
}
