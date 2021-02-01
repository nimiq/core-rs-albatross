use futures::{
    future,
    io::{AsyncRead, AsyncWrite},
};
use libp2p::{core::UpgradeInfo, identity::Keypair, InboundUpgrade, Multiaddr, OutboundUpgrade};
use rand::{thread_rng, RngCore};

use beserial::{Deserialize, Serialize, SerializingError};
use nimiq_hash::Blake2bHash;
use nimiq_macros::{add_hex_io_fns_typed_arr, create_typed_array};
use nimiq_utils::tagged_signing::{TaggedSignable, TaggedSignature};

use super::peer_contacts::{Protocols, Services, SignedPeerContact};
use crate::{
    message_codec::{MessageReader, MessageWriter},
    DISCOVERY_PROTOCOL,
};

create_typed_array!(ChallengeNonce, u8, 32);
add_hex_io_fns_typed_arr!(ChallengeNonce, ChallengeNonce::SIZE);

impl ChallengeNonce {
    pub fn generate() -> Self {
        let mut nonce = Self::default();

        thread_rng().fill_bytes(&mut nonce.0);

        nonce
    }
}

impl TaggedSignable for ChallengeNonce {
    const TAG: u8 = 0x01;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[repr(u8)]
pub enum DiscoveryMessage {
    #[beserial(discriminant = 1)]
    Handshake {
        /// The addresses of the receiver as observed by the sender.
        #[beserial(len_type(u8))]
        observed_addresses: Vec<Multiaddr>,

        /// The challenge that the receiver must use for the response in `HandshakeAck`.
        challenge_nonce: ChallengeNonce,

        /// Genesis hash for the network the sender is in.
        genesis_hash: Blake2bHash,

        /// Number of peer contacts the sender is willing to accept per update.
        limit: u16,

        /// Service flags for which the sender needs peer contacts.
        services: Services,

        /// Protocol flags for which the sender needs peer contacts.
        protocols: Protocols,

        /// User agent string of the sender.
        #[beserial(len_type(u8))]
        user_agent: String,
    },

    #[beserial(discriminant = 2)]
    HandshakeAck {
        /// Peer contact of the sender
        peer_contact: SignedPeerContact,

        /// Signature for the challenge sent in `HandshakeAck`, signed with the identity keypair (same one as used for
        /// the peer contact).
        response_signature: TaggedSignature<ChallengeNonce, Keypair>,

        /// Interval in ms in which the peer wants to receive new updates.
        update_interval: Option<u64>,

        /// Initial set of peer contacts.
        #[beserial(len_type(u16))]
        peer_contacts: Vec<SignedPeerContact>,
    },

    #[beserial(discriminant = 3)]
    PeerAddresses {
        #[beserial(len_type(u16))]
        peer_contacts: Vec<SignedPeerContact>,
    },
}

/// # TODO
///
///  - Instead of using an enum for `DiscoveryMessage`, we could have a struct for each variant. The upgrade then
///    returns a `MessageReader<Handshake>`. The protocol handler can then first read the Handshake and convert the
///    stream to a `MessageReader<HandshakeAck>` and so forth. The specific streams then need to be put into the
///    handler's state enum.
///
pub struct DiscoveryProtocol;

impl UpgradeInfo for DiscoveryProtocol {
    type Info = &'static [u8];
    type InfoIter = std::iter::Once<Self::Info>;

    fn protocol_info(&self) -> Self::InfoIter {
        std::iter::once(DISCOVERY_PROTOCOL)
    }
}

impl<C> InboundUpgrade<C> for DiscoveryProtocol
where
    C: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    type Output = MessageReader<C, DiscoveryMessage>;
    type Error = SerializingError;
    type Future = future::Ready<Result<Self::Output, Self::Error>>;

    fn upgrade_inbound(self, socket: C, info: Self::Info) -> Self::Future {
        log::trace!("upgrade_inbound: {:?}", info);
        future::ok(MessageReader::new(socket))
    }
}

impl<C> OutboundUpgrade<C> for DiscoveryProtocol
where
    C: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    type Output = MessageWriter<C, DiscoveryMessage>;
    type Error = SerializingError;
    type Future = future::Ready<Result<Self::Output, Self::Error>>;

    fn upgrade_outbound(self, socket: C, info: Self::Info) -> Self::Future {
        log::trace!("upgrade_outbound: {:?}", info);
        future::ok(MessageWriter::new(socket))
    }
}
