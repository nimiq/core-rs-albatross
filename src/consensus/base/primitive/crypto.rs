//#[derive(PartialEq,Eq)]
//pub struct PublicKey ([u8; 32]);

create_typed_array!(PublicKey, u8, 32);
create_typed_array!(PrivateKey, u8, 32);
create_typed_array!(Commitment, u8, 32);
create_typed_array!(RandomSecret, u8, 32);

impl PublicKey {
    pub fn derive<'a>(private_key: &PrivateKey) -> &'a Self {
        unimplemented!()
    }

    pub fn sum(public_keys: Vec<PublicKey>) -> Self {
        unimplemented!()
    }
}

impl From<PrivateKey> for PublicKey {
    fn from(private_key: PrivateKey) -> Self {
        unimplemented!()
    }
}

impl PrivateKey {
    pub fn generate<'a>() -> &'a Self {
        unimplemented!()
    }
}

#[derive(PartialEq,Eq)]
pub struct KeyPair<'a> {
    private_key: &'a PrivateKey,
    public_key: &'a PublicKey
}

impl<'a> KeyPair<'a> {
    pub fn generate() -> Self {
        return PrivateKey::generate().into();
    }
}

impl<'a> From<&'a PrivateKey> for KeyPair<'a> {
    fn from(private_key: &'a PrivateKey) -> Self {
        return KeyPair { private_key, public_key: PublicKey::derive(private_key) };
    }
}

pub struct Signature ([u8; 64]);
impl Eq for Signature { }
impl PartialEq for Signature {
    fn eq(&self, other: &Signature) -> bool {
        return self.0[0..64] == other.0[0..64];
    }
}

impl Signature {
    pub fn create<'a, Key: Into<KeyPair<'a>>>(key: Key, data: &[u8]) -> Self {
        let key_pair: KeyPair = key.into();
        unimplemented!()
    }
}

pub fn signature_verify(signature: Signature, public_key: PublicKey, data: &[u8]) -> bool {
    unimplemented!()
}

#[derive(PartialEq,Eq)]
pub struct CommitmentPair<'a> {
    random_secret: &'a RandomSecret,
    commitment: &'a Commitment
}
