use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_keys::{Address, KeyPair};
use nimiq_network_interface::network::Network as NetworkInterface;
use nimiq_network_libp2p::{
    dht::{DhtRecord, DhtVerifierError, Verifier as DhtVerifier},
    discovery::peer_contacts::{ValidatorInfoError, ValidatorRecordVerifier},
    libp2p::kad::Record,
    Network, PeerId,
};
use nimiq_serde::Deserialize;
use nimiq_utils::tagged_signing::{TaggedSignable, TaggedSigned};
use nimiq_validator_network::validator_record::ValidatorRecord;

pub struct Verifier {
    blockchain: BlockchainProxy,
}

impl Verifier {
    pub fn new(blockchain: BlockchainProxy) -> Self {
        Self { blockchain }
    }
}

impl ValidatorRecordVerifier for Verifier {
    fn verify_validator_record(
        &self,
        signed_record: &TaggedSigned<
            ValidatorRecord<<Network as NetworkInterface>::PeerId>,
            KeyPair,
        >,
    ) -> Result<(), ValidatorInfoError> {
        // // Deserialize the value of the record, which is a ValidatorRecord. If it fails return an error.
        // let validator_record =
        //     TaggedSigned::<ValidatorRecord<PeerId>, KeyPair>::deserialize_from_vec(&record.record.value)
        //         .map_err(DhtVerifierError::MalformedValue)?;

        // // Make sure the peer who signed the record is also the one presented in the record.
        // if let Some(publisher) = record.publisher {
        //     if validator_record.record.peer_id != publisher {
        //         return Err(DhtVerifierError::PublisherMismatch(
        //             publisher,
        //             validator_record.record.peer_id,
        //         ));
        //     }
        // } else {
        //     log::warn!("Validating a dht record without a publisher");
        //     return Err(DhtVerifierError::PublisherMissing);
        // }

        // // Deserialize the key of the record which is an Address. If it fails return an error.
        // let validator_address = Address::deserialize_from_vec(record.key.as_ref())
        //     .map_err(DhtVerifierError::MalformedKey)?;

        // Make sure the validator address used as key is identical to the one in the record.
        // if signed_record.record.validator_address != validator_address {
        //     return Err(DhtVerifierError::AddressMismatch(
        //         validator_address,
        //         validator_record.record.validator_address,
        //     ));
        // }

        // Acquire blockchain read access. For now exclude Light clients.
        let blockchain = match self.blockchain {
            BlockchainProxy::Light(ref _light_blockchain) => {
                panic!("Light Blockchains cannot verify validator records.")
            }
            BlockchainProxy::Full(ref full_blockchain) => full_blockchain,
        };
        let blockchain_read = blockchain.read();

        // Get the staking contract to retrieve the public key for verification.
        let staking_contract = blockchain_read
            .get_staking_contract_if_complete(None)
            .ok_or(ValidatorInfoError::StateIncomplete)?;

        // Get the public key needed for verification.
        let data_store = blockchain_read.get_staking_contract_store();
        let txn = blockchain_read.read_transaction();
        let public_key = staking_contract
            .get_validator(
                &data_store.read(&txn),
                &signed_record.record.validator_address,
            )
            .ok_or(ValidatorInfoError::UnknownValidator(
                signed_record.record.validator_address.clone(),
            ))?
            .signing_key;

        // Verify the record.
        signed_record
            .verify(&public_key)
            .then(|| ())
            .ok_or(ValidatorInfoError::InvalidSignature)
    }
}

impl DhtVerifier for Verifier {
    fn verify(&self, record: &Record) -> Result<DhtRecord, DhtVerifierError> {
        // Peek the tag to know what kind of record this is.
        let Some(tag) = TaggedSigned::<ValidatorRecord<PeerId>, KeyPair>::peek_tag(&record.value)
        else {
            log::warn!(?record, "DHT Tag not peekable.");
            return Err(DhtVerifierError::MalformedTag);
        };

        // Depending on tag perform the verification.
        match tag {
            ValidatorRecord::<PeerId>::TAG => {
                // Deserialize the value of the record, which is a ValidatorRecord. If it fails return an error.
                let validator_record =
                    TaggedSigned::<ValidatorRecord<PeerId>, KeyPair>::deserialize_from_vec(
                        &record.value,
                    )
                    .map_err(DhtVerifierError::MalformedValue)?;

                // Make sure the peer who published the record is also the one signed into the record.
                if record.publisher.ok_or(DhtVerifierError::MissingPublisher)?
                    != validator_record.record.peer_id
                {
                    return Err(DhtVerifierError::PublisherMismatch(
                        record.publisher.unwrap(),
                        validator_record.record.peer_id,
                    ));
                }

                // Deserialize the key of the record which is an Address. If it fails return an error.
                let validator_address = Address::deserialize_from_vec(record.key.as_ref())
                    .map_err(DhtVerifierError::MalformedKey)?;

                // Make sure the address used as key is also the one signed into the record.
                if validator_address != validator_record.record.validator_address {
                    return Err(DhtVerifierError::AddressMismatch(
                        validator_address,
                        validator_record.record.validator_address,
                    ));
                }

                self.verify_validator_record(&validator_record)
                    .map_err(DhtVerifierError::ValidatorInfoError)
                    .and_then(|_| {
                        Ok(DhtRecord::Validator(
                            validator_record.record.peer_id,
                            validator_record.record,
                            record.clone(),
                        ))
                    })
            }
            _ => {
                log::error!(tag, "DHT invalid record tag received");
                Err(DhtVerifierError::UnknownTag)
            }
        }
    }
}
