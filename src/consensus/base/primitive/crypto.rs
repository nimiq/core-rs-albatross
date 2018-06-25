extern crate ed25519_dalek;
extern crate curve25519_dalek;
extern crate rand;
extern crate sha2;

use self::rand::{OsRng, Rng};
use self::sha2::Digest;
use self::curve25519_dalek::scalar::Scalar;
use self::curve25519_dalek::edwards::EdwardsPoint;

create_typed_array!(PublicKey, u8, 32);
create_typed_array!(PrivateKey, u8, 32);

impl PublicKey {
    pub fn sum(public_keys: Vec<PublicKey>) -> Self {
        unimplemented!()
    }
}

impl<'a> From<&'a PrivateKey> for PublicKey {
    fn from(private_key: &'a PrivateKey) -> Self {
        let secret_key = ed25519_dalek::SecretKey::from_bytes(&private_key.0).unwrap();
        let public_key = ed25519_dalek::PublicKey::from_secret::<sha2::Sha512>(&secret_key);
        return PublicKey(public_key.to_bytes());
    }
}

impl PrivateKey {
    pub fn generate() -> Self {
        let mut cspring: OsRng = OsRng::new().unwrap();
        return PrivateKey(ed25519_dalek::SecretKey::generate(&mut cspring).to_bytes());
    }
}

#[derive(PartialEq,Eq)]
pub struct KeyPair {
    private_key: PrivateKey,
    public_key: PublicKey
}

impl KeyPair {
    pub fn generate() -> Self {
        return PrivateKey::generate().into();
    }
}

impl From<PrivateKey> for KeyPair {
    fn from(private_key: PrivateKey) -> Self {
        return KeyPair { public_key: PublicKey::from(&private_key), private_key };
    }
}

pub struct Signature ([u8; 64]);
impl Eq for Signature { }
impl PartialEq for Signature {
    fn eq(&self, other: &Signature) -> bool {
        return self.0[0..64] == other.0[0..64];
    }
}

pub fn signature_create<Key: Into<KeyPair>>(key: Key, data: &[u8]) -> Signature {
    let key_pair: KeyPair = key.into();
    let ext_sec_key = ed25519_dalek::SecretKey::from_bytes(&key_pair.private_key.0).unwrap();
    let ext_pub_key = ed25519_dalek::PublicKey::from_bytes(&key_pair.public_key.0).unwrap();
    let ext_key_pair = ed25519_dalek::Keypair { secret: ext_sec_key, public: ext_pub_key };
    let ext_signature = ext_key_pair.sign::<sha2::Sha512>(data);
    return Signature(ext_signature.to_bytes());
}

pub fn signature_verify(signature: Signature, public_key: PublicKey, data: &[u8]) -> bool {
    let ext_pub_key = ed25519_dalek::PublicKey::from_bytes(&public_key.0).unwrap();
    let ext_signature = ed25519_dalek::Signature::from_bytes(&signature.0).unwrap();
    return ext_pub_key.verify::<sha2::Sha512>(data, &ext_signature);
}

#[derive(PartialEq,Eq)]
pub struct RandomSecret(Scalar);
#[derive(PartialEq,Eq)]
pub struct Commitment(EdwardsPoint);

#[derive(PartialEq,Eq)]
pub struct CommitmentPair {
    random_secret: RandomSecret,
    commitment: Commitment
}

impl CommitmentPair {
    pub fn generate() -> Option<CommitmentPair> {
        // Create random 32 bytes.
        let mut cspring: OsRng = OsRng::new().unwrap();
        let mut randomness: [u8; 32] = [0u8; 32];
        cspring.fill_bytes(&mut randomness);

        // Decompress the 32 byte cryptographically secure random data to 64 byte.
        let mut h: sha2::Sha512 = sha2::Sha512::default();
        let mut hash:  [u8; 64] = [0u8; 64];

        h.input(&randomness);
        hash.copy_from_slice(h.result().as_slice());

        // Reduce to valid scalar.
        let scalar = Scalar::from_bytes_mod_order_wide(&hash);
        if scalar == Scalar::zero() || scalar == Scalar::one() {
            return None;
        }

        // Compute the point [secret]B. Let the string R be the encoding of this point.
        let commitment = scalar * curve25519_dalek::constants::ED25519_BASEPOINT_POINT;

        let rs = RandomSecret(scalar);
        let ct = Commitment(commitment);
        return Some(CommitmentPair { random_secret: rs, commitment: ct });
    }
}
