use url::Url;

use nimiq_keys::PublicKey;

#[derive(Clone, Debug)]
pub struct SeedList {
    url: Url,
    public_key: Option<PublicKey>,
}

impl SeedList {
    pub fn new(url: Url, public_key: Option<PublicKey>) -> Self {
        Self { url, public_key }
    }

    pub fn url(&self) -> &Url {
        &self.url
    }

    pub fn public_key(&self) -> &Option<PublicKey> {
        &self.public_key
    }
}
