use keys::PublicKey;

pub struct SeedListUrl {
    url: String,
    public_key: Option<PublicKey>,
}

impl SeedListUrl {
    pub fn new(url: String, public_key: Option<PublicKey>) -> Self {
        Self {
            url,
            public_key,
        }
    }

    pub fn url(&self) -> &String {
        &self.url
    }

    pub fn public_key(&self) -> &Option<PublicKey> {
        &self.public_key
    }
}
