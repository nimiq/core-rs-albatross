use std::io::{BufRead, Error as IoError};
use std::str::FromStr;
use std::sync::Arc;

use hex::FromHex;
use failure::Fail;
use futures::{future::*, Future, Stream};
use parking_lot::Mutex;
use reqwest::r#async::{Chunk, Client, Response};

use keys::Signature;
use network_primitives::address::peer_address::PeerAddress;
use network_primitives::address::peer_uri::{PeerUri, PeerUriError};
use network_primitives::networks::{
    create_seed_peer_addr,
    create_seed_peer_addr_ws,
    get_network_info,
    NetworkId
};
use network_primitives::protocol::Protocol;
use utils::observer::Notifier;

pub enum PeerAddressSeederEvent {
    Seeds(Vec<PeerAddress>),
    End,
}

#[derive(Fail, Debug)]
pub enum PeerAddressSeederError {
    #[fail(display = "The seed list file didn't contain any parseable seed peer address")]
    EmptySeedAddresses,
    #[fail(display = "The fetching of the seed list file failed with error '{}'", _0)]
    FetchError(#[cause] reqwest::Error),
    #[fail(display = "Failed while reading a line from the seed list with io::error '{}'", _0)]
    IoError(IoError),
    #[fail(display = "Seed node address could not be parsed into a PeerUri")]
    PeerUriParsingError(#[cause] PeerUriError),
    #[fail(display = "The seed node didn't provide any parseable public key")]
    SeedNodeMissingPublicKey,
    #[fail(display = "The seed list file didn't contain any parseable signature")]
    SignatureMissing,
    #[fail(display = "The signature in the file was in a line other than the last one")]
    SignatureNotInLastLine,
    #[fail(display = "The signature verification for the seed list file failed")]
    SignatureVerificationFailed,
    #[fail(display = "The remote server responded with status code '{}'", _0)]
    UnexpectedHttpStatus(reqwest::StatusCode),
}

impl From<reqwest::Error> for PeerAddressSeederError {
    fn from(error: reqwest::Error) -> Self {
        PeerAddressSeederError::FetchError(error)
    }
}

impl From<PeerUriError> for PeerAddressSeederError {
    fn from(error: PeerUriError) -> Self {
        PeerAddressSeederError::PeerUriParsingError(error)
    }
}

pub struct PeerAddressSeeder {
    pub notifier: Arc<Mutex<Notifier<'static, PeerAddressSeederEvent>>>,
}

impl PeerAddressSeeder {
    pub fn new() -> Self {
        Self {
            notifier: Arc::new(Mutex::new(Notifier::new())),
        }
    }

    pub fn collect(&self, network_id: NetworkId) {
        let network_info = get_network_info(network_id).expect("This was validated by PeerAddressBook::new()");

        // Process all seed lists asynchronously
        for seed_list in &network_info.seed_lists {
            let notifier = Arc::clone(&self.notifier);
            let seed_list_url = seed_list.url().clone();

            let task = Self::fetch(seed_list.url())
            .and_then(move |response_body| {
                let mut signature = None;
                let mut seed_addresses = Vec::new();

                // Process each line of the seed list
                for line in response_body.lines() {
                    // Abort if the line can't be read properly
                    if line.is_err() {
                        return err(PeerAddressSeederError::IoError(line.unwrap_err()));
                    }
                    let line = line.expect("Validated this above");

                    // The signature should always be in the last line
                    if signature.is_some() {
                        return err(PeerAddressSeederError::SignatureNotInLastLine);
                    }

                    // Ignore comments and empty lines
                    let line = line.trim();
                    if line.is_empty() || line.starts_with("#") {
                        continue;
                    }

                    // Try to parse the line as a seed address, if that fails, fallback to try to parse it as a signature
                    // TODO: Should we fail if this step fails (i.e. if there is a non-comment/non-empty line that is not
                    // a seed address neither a signature)?
                    let seed_address = Self::str_to_seed_peer_addr(&line);
                    match seed_address {
                        Ok(seed_address) => seed_addresses.push(seed_address),
                        _ => signature = Signature::from_hex(line).ok(),
                    }
                }

                // Error out if we couldn't find any parseable seed address
                if seed_addresses.is_empty() {
                    return err(PeerAddressSeederError::EmptySeedAddresses);
                }

                // Verify the signature if a public key was provided for this seed list
                if let Some(public_key) = seed_list.public_key() {
                    if let Some(signature) = signature {
                        // Serialize the seed addresses for signature verification
                        let data = seed_addresses.iter().filter_map(|seed| seed.to_seed_string()).collect::<Vec<String>>().join("\n");
                        let data = data.as_bytes();

                        if !public_key.verify(&signature, data) {
                            return err(PeerAddressSeederError::SignatureVerificationFailed)
                        }
                    } else { // No signature was found on the seed list file
                        return err(PeerAddressSeederError::SignatureMissing);
                    }
                }

                // Notify the Seeds event with the array of seed addresses
                notifier.lock().notify(PeerAddressSeederEvent::Seeds(seed_addresses));
                ok(())
            })
            .map_err(move |err| warn!("Failed to retrieve seed list from {}: {}", seed_list_url, err));

            tokio::spawn(task);
        }
        // Notify that we're done collecting seed addresses
        self.notifier.lock().notify(PeerAddressSeederEvent::End);
    }

    // Asynchronously fetches a seed list from a remote location
    fn fetch(url: &str) -> impl Future<Item=Chunk, Error=PeerAddressSeederError> {
        Client::new().get(url).send()
        .map_err(PeerAddressSeederError::from)
        .and_then(Self::fetch_callback)
    }

    fn str_to_seed_peer_addr(line: &str) -> Result<PeerAddress, PeerAddressSeederError> {
        let uri = PeerUri::from_str(line)?;

        // TODO: May be we want to allow seed nodes without public key?
        // Note: only Wss and Ws nodes have a public key, so this also catches other node types
        if uri.public_key().is_none() {
            return Err(PeerAddressSeederError::SeedNodeMissingPublicKey);
        }

        match uri.protocol() {
            Protocol::Wss => Ok(create_seed_peer_addr(uri.hostname().unwrap(), uri.port().unwrap_or(443), uri.public_key().unwrap())),
            Protocol::Ws => Ok(create_seed_peer_addr_ws(uri.hostname().unwrap(), uri.port().unwrap_or(80), uri.public_key().unwrap())),
            _ => unreachable!(), // Check the previous comment
        }
    }

    // Note: this is a standalone function to help the compiler because as a closure in the fetch() function
    // it would fail to infer the types correctly
    fn fetch_callback(res: Response) -> Box<Future<Item=Chunk, Error=PeerAddressSeederError> + Send> {
        let status = res.status();
        if status == 200 {
            Box::new(res.into_body().concat2().map_err(PeerAddressSeederError::from))
        } else {
            Box::new(err(PeerAddressSeederError::UnexpectedHttpStatus(status)))
        }
    }
}

