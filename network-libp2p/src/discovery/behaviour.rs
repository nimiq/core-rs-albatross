use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use futures::StreamExt;
use instant::SystemTime;
use libp2p::{
    core::{transport::PortUse, Endpoint},
    identity::Keypair,
    swarm::{
        behaviour::{ConnectionClosed, ConnectionEstablished},
        CloseConnection, ConnectionDenied, ConnectionId, FromSwarm, NetworkBehaviour, ToSwarm,
    },
    Multiaddr, PeerId,
};
use nimiq_hash::Blake2bHash;
use nimiq_keys::Address;
use nimiq_network_interface::peer_info::Services;
use nimiq_time::{interval, Interval};
use parking_lot::RwLock;

use super::{
    handler::{Handler, HandlerOutEvent},
    peer_contacts::{PeerContact, PeerContactBook, PeerContactInfo},
    validator_verifier::{ValidatorClaimBudget, ValidatorClaimVerifier, ValidatorVerification},
};

#[derive(Clone, Debug)]
pub struct Config {
    /// Genesis hash for the network we want to be connected to.
    pub genesis_hash: Blake2bHash,

    /// Interval in which we want to be updated.
    pub update_interval: Duration,

    /// Minimum update interval, that we will accept. If peer contact updates are received faster than this, they will
    /// be rejected.
    pub min_recv_update_interval: Duration,

    /// How many updated peer contacts we want to receive per update.
    pub update_limit: u16,

    /// Services for which we filter (the services that we need others to provide)
    pub required_services: Services,

    /// Minimum interval that we will update other peers with.
    pub min_send_update_interval: Duration,

    /// Interval in which the peer address book is cleaned up.
    pub house_keeping_interval: Duration,

    /// Whether to keep the connection alive, even if no other behaviour uses it.
    pub keep_alive: bool,

    /// Only secure websocket connections
    pub only_secure_ws_connections: bool,
}

impl Config {
    pub fn new(
        genesis_hash: Blake2bHash,
        required_services: Services,
        only_secure_ws_connections: bool,
    ) -> Self {
        Self {
            genesis_hash,
            update_interval: Duration::from_secs(60),
            min_send_update_interval: Duration::from_secs(30),
            min_recv_update_interval: Duration::from_secs(30),
            update_limit: 64,
            required_services,
            house_keeping_interval: Duration::from_secs(60),
            keep_alive: true,
            only_secure_ws_connections,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Event {
    Established {
        peer_id: PeerId,
        peer_address: Multiaddr,
        peer_contact: PeerContact,
    },
    Update,
}

type DiscoveryToSwarm = ToSwarm<Event, ()>;

/// Network behaviour for peer exchange.
///
/// When a connection to a peer is established, a handshake is done to exchange protocols and services filters, and
/// subscription settings. The peers then send updates to each other in a configurable interval.
///
/// # TODO
///
///  - Exchange clock time with other peers.
///
pub struct Behaviour {
    /// Configuration for the discovery behaviour
    config: Config,

    /// Identity key pair
    keypair: Keypair,

    /// `PeerId`s of all connected peers.
    connected_peers: HashSet<PeerId>,

    /// Contains all known peer contacts.
    peer_contact_book: Arc<RwLock<PeerContactBook>>,

    /// Queue with events to emit.
    pub events: VecDeque<DiscoveryToSwarm>,

    /// Timer to do house-keeping in the peer address book.
    house_keeping_timer: Interval,

    /// Checks the validator claims carried by peer contacts.
    validator_verifier: Arc<dyn ValidatorClaimVerifier>,

    /// Bounds how many validator claims are verified per house-keeping tick, across every
    /// discovery connection combined. Shared with each [`Handler`].
    validator_claim_budget: Arc<ValidatorClaimBudget>,

    /// Rotating offset into the peer-ID-ordered list returned by
    /// [`PeerContactBook::unverified_validator_contacts`].
    ///
    /// A backlog of contacts needing a re-check can persist across many ticks (e.g. right after
    /// this node's staking-contract view becomes complete, when everything that was
    /// `Unverifiable(StateIncomplete)` becomes due for its first real check at once). Always
    /// taking the same fixed prefix of that list every tick would let contacts past
    /// `MAX_RECHECKED_CLAIMS_PER_TICK` starve indefinitely, delaying both promotion of valid claims
    /// and revocation of previous verified bindings. Advancing this cursor by the tick's window
    /// size guarantees every contact is reached within
    /// `ceil(backlog / MAX_RECHECKED_CLAIMS_PER_TICK)` ticks.
    validator_recheck_cursor: usize,

    /// Rotating offset into the peer-ID-ordered list returned by
    /// [`PeerContactBook::stale_verified_validator_contacts`], for the same reason as
    /// [`Self::validator_recheck_cursor`].
    stale_binding_recheck_cursor: usize,
}

impl Behaviour {
    /// Maximum number of validator claims re-checked per house-keeping tick.
    ///
    /// Re-checking reads blockchain state, and house-keeping runs on the swarm task, so this
    /// bounds how long a single tick can hold it up. The sweep stops early, at the first claim
    /// that cannot be verified for a node-wide reason (e.g. while this node is still syncing its
    /// staking contract), since every other claim would come back the same way. See
    /// [`Self::recheck_validator_claims`].
    const MAX_RECHECKED_CLAIMS_PER_TICK: usize = 128;

    /// Maximum number of validator claims verified per house-keeping tick across all discovery
    /// connections combined, when they arrive via a handshake or a peer-address update. This is
    /// the general part of the shared [`ValidatorClaimBudget`], which any claim can spend.
    ///
    /// Unlike the re-check above, this path is driven directly by untrusted peers: the very
    /// first message on a new connection can carry up to `Config::update_limit` claims, and
    /// nothing else bounds how many connections can be open at once. Claims beyond the budget
    /// are left unverified and picked up by the re-check sweep on a later tick. Once a check
    /// (inbound or re-check) shows that this node cannot verify any claim right now, the budget
    /// is exhausted until the next tick.
    const MAX_INBOUND_CLAIMS_VERIFIED_PER_TICK: usize = 256;

    /// Number of validator claims per house-keeping tick, on top of
    /// [`Self::MAX_INBOUND_CLAIMS_VERIFIED_PER_TICK`], reserved for refreshed contacts of peers
    /// that already hold a live verified binding to the validator address they claim (see
    /// [`ValidatorClaimBudget::try_consume_refresh`]).
    ///
    /// Without it, an unauthenticated peer could use up the general budget with a couple of
    /// connections full of bogus claims under fresh peer keys. An honest validator's refreshed
    /// contact would then be stored pending, and so not gossiped, which would stop this node from
    /// relaying validator contacts altogether.
    ///
    /// Each validator address takes at most one unit of it per tick, which is what honest
    /// refreshes need: each validator re-signs its contact once per house-keeping tick, and relays
    /// of a contact we already have are not checked again (see [`PeerContactBook::is_new`]).
    /// Copies of a refresh that are checked before the first one is stored, e.g. several in one
    /// batch or on several connections at once, are thus only charged to the reserve once; the
    /// others fall back to the general budget, like any refresh beyond the reserve. Only a
    /// validator's signing key can obtain a binding, and relaying the validator's genuine refreshed
    /// contact to us at most spends that validator's unit on verifying it. Draining the reserve
    /// thus takes as many staked validators as it has units, however many peer IDs each of them
    /// binds, and even that only pushes other refreshes back to the general budget. In the worst
    /// case, discovery reads blockchain state `MAX_INBOUND_CLAIMS_VERIFIED_PER_TICK +
    /// INBOUND_REFRESH_CLAIM_RESERVE_PER_TICK + MAX_RECHECKED_CLAIMS_PER_TICK +
    /// MAX_RECHECKED_STALE_BINDINGS_PER_TICK` = 256 + 256 + 128 + 32 times per tick.
    const INBOUND_REFRESH_CLAIM_RESERVE_PER_TICK: usize = 256;

    /// Number of distinct peers claiming the same validator address that can verify a contact per
    /// tick, one contact each, on every path that checks claims (see
    /// [`ValidatorClaimBudget::may_verify`]).
    ///
    /// An honest validator re-signs its contact once per tick, and binds a new peer only when it
    /// starts or moves to another node, so it needs one or two. A validator can sign claims for as
    /// many peer IDs as it likes, though, and refresh each of them every second. Its verified
    /// contacts are gossiped, so without this limit a single one could spend the general budget of
    /// every node, keeping the first claims of other validators from being verified.
    const VERIFIED_CLAIMS_PER_VALIDATOR_PER_TICK: usize = 4;

    /// Maximum number of stale verified bindings re-checked per house-keeping tick, on top of
    /// [`Self::MAX_RECHECKED_CLAIMS_PER_TICK`]. See
    /// [`PeerContactBook::stale_verified_validator_contacts`].
    ///
    /// Each validator address keeps at most [`PeerContactBook::MAX_BINDINGS_PER_VALIDATOR`]
    /// bindings, and online validators refresh theirs every tick, so the stale ones are few.
    const MAX_RECHECKED_STALE_BINDINGS_PER_TICK: usize = 32;

    pub fn new(
        config: Config,
        keypair: Keypair,
        peer_contact_book: Arc<RwLock<PeerContactBook>>,
        validator_verifier: Arc<dyn ValidatorClaimVerifier>,
    ) -> Self {
        let house_keeping_timer = interval(config.house_keeping_interval);
        peer_contact_book.write().update_own_contact(&keypair);

        // Report our own known addresses as candidates to the swarm
        let mut events = VecDeque::new();
        for address in peer_contact_book.read().get_own_contact().addresses() {
            events.push_back(ToSwarm::NewExternalAddrCandidate(address.clone()));
        }

        Self {
            config,
            keypair,
            connected_peers: HashSet::new(),
            peer_contact_book,
            events,
            house_keeping_timer,
            validator_verifier,
            validator_claim_budget: Arc::new(
                ValidatorClaimBudget::with_refresh_reserve(
                    Self::MAX_INBOUND_CLAIMS_VERIFIED_PER_TICK,
                    Self::INBOUND_REFRESH_CLAIM_RESERVE_PER_TICK,
                )
                .with_per_validator_limit(Self::VERIFIED_CLAIMS_PER_VALIDATOR_PER_TICK),
            ),
            validator_recheck_cursor: 0,
            stale_binding_recheck_cursor: 0,
        }
    }

    /// Pure windowing math behind [`Self::select_recheck_window`]: given a pool of `len` items
    /// ordered deterministically and a rotating `cursor`, returns the `(start, window_len)` of
    /// the slice-with-wraparound to select this tick, so that repeated calls with `cursor`
    /// advanced by the returned `window_len` each time eventually cover every index at least
    /// once, however large `len` grows relative to `max_window`.
    fn recheck_window_bounds(cursor: usize, len: usize, max_window: usize) -> (usize, usize) {
        if len == 0 {
            return (0, 0);
        }
        (cursor % len, max_window.min(len))
    }

    /// Selects up to `MAX_RECHECKED_CLAIMS_PER_TICK` contacts to re-check this tick from `pool`
    /// (must be sorted deterministically, as [`PeerContactBook::unverified_validator_contacts`]
    /// already is), rotating the window on every call via `self.validator_recheck_cursor`.
    ///
    /// This is the fairness step described on [`Self::validator_recheck_cursor`]: without it, a
    /// backlog above the per-tick cap would always re-check the same prefix. Returns owned clones
    /// (an `Arc` bump each) rather than borrowing `pool`, so the mutable borrow of `self` this
    /// takes doesn't linger into the caller's subsequent use of `self.validator_verifier`.
    ///
    /// It takes at most `VERIFIED_CLAIMS_PER_VALIDATOR_PER_TICK` contacts claiming the same
    /// validator, as no more of them can verify this tick (see
    /// [`ValidatorClaimBudget::may_verify`]). The rest of them would only take up the window.
    fn select_recheck_window(
        &mut self,
        pool: &[Arc<PeerContactInfo>],
    ) -> Vec<Arc<PeerContactInfo>> {
        Self::select_window(
            &mut self.validator_recheck_cursor,
            pool,
            Self::MAX_RECHECKED_CLAIMS_PER_TICK,
            Self::VERIFIED_CLAIMS_PER_VALIDATOR_PER_TICK,
        )
    }

    /// Like [`Self::select_recheck_window`], for the stale verified bindings returned by
    /// [`PeerContactBook::stale_verified_validator_contacts`], rotating
    /// `self.stale_binding_recheck_cursor` and taking up to `MAX_RECHECKED_STALE_BINDINGS_PER_TICK`.
    fn select_stale_binding_window(
        &mut self,
        pool: &[Arc<PeerContactInfo>],
    ) -> Vec<Arc<PeerContactInfo>> {
        Self::select_window(
            &mut self.stale_binding_recheck_cursor,
            pool,
            Self::MAX_RECHECKED_STALE_BINDINGS_PER_TICK,
            usize::MAX,
        )
    }

    /// Takes up to `max_window` items of `pool`, starting at `cursor` (with wraparound) and
    /// skipping the items claiming a validator that already has `per_validator` items in the
    /// window, and advances `cursor`. See [`Self::recheck_window_bounds`].
    ///
    /// If the window fills up, `cursor` moves past the items it looked at. Otherwise every item
    /// that was not skipped is in the window, and `cursor` moves to the first skipped item, so
    /// that the next window starts with it. Moving past all of them instead would start every
    /// window at the same item, and always skip the same items of a validator with more than
    /// `per_validator` of them.
    fn select_window(
        cursor: &mut usize,
        pool: &[Arc<PeerContactInfo>],
        max_window: usize,
        per_validator: usize,
    ) -> Vec<Arc<PeerContactInfo>> {
        let (start, _) = Self::recheck_window_bounds(*cursor, pool.len(), max_window);
        let mut window = Vec::new();
        let mut per_address: HashMap<&Address, usize> = HashMap::new();
        let mut looked_at = 0;
        let mut first_skipped = None;
        for contact in pool.iter().cycle().skip(start).take(pool.len()) {
            if window.len() == max_window {
                break;
            }
            looked_at += 1;
            if let Some(address) = contact.validator_address() {
                let taken = per_address.entry(address).or_default();
                if *taken >= per_validator {
                    first_skipped.get_or_insert(looked_at - 1);
                    continue;
                }
                *taken += 1;
            }
            window.push(Arc::clone(contact));
        }
        let advance = if window.len() == max_window {
            looked_at
        } else {
            first_skipped.unwrap_or(looked_at)
        };
        *cursor = cursor.wrapping_add(advance);
        window
    }

    /// Re-checks the validator claims of `window` (see [`Self::select_recheck_window`]) in order,
    /// returning the outcomes for [`PeerContactBook::apply_validator_verifications`].
    ///
    /// Checking reads blockchain state, so this must be called without holding the contact book
    /// lock. It stops at the first claim that cannot be verified for a node-wide reason (see
    /// [`UnverifiableReason::is_node_wide`](super::validator_verifier::UnverifiableReason::is_node_wide)),
    /// e.g. while this node is still syncing its staking contract: every other claim in the
    /// window would come back the same way, each at the cost of another blockchain read on the
    /// swarm task. It then also exhausts `budget`, so that the discovery connections stop checking
    /// inbound claims until the next tick as well. The outcomes computed up to that point are
    /// still returned.
    ///
    /// Like on arrival, a claim is only checked while the validator it claims has not used up its
    /// share of verified claims for this tick, and takes part of it if it verifies (see
    /// [`ValidatorClaimBudget::may_verify`]).
    fn recheck_validator_claims(
        window: &[Arc<PeerContactInfo>],
        verifier: &dyn ValidatorClaimVerifier,
        budget: &ValidatorClaimBudget,
    ) -> Vec<(PeerId, u64, ValidatorVerification)> {
        let mut verifications = Vec::with_capacity(window.len());
        for contact in window {
            let Some(validator_address) = contact.validator_address() else {
                continue;
            };
            // A contact that was verified before only has its stale binding re-checked, which can
            // end the binding but never renew it (see
            // `PeerContactBook::apply_validator_verifications`), so it does not count towards the
            // validator's share.
            let (peer_id, timestamp) = (*contact.peer_id(), contact.contact().timestamp());
            let pending = contact.validator_claim_needs_recheck();
            if pending && !budget.may_verify(validator_address, &peer_id, timestamp) {
                continue;
            }
            let Some(verification) = contact.signed().check_validator_claim(verifier) else {
                continue;
            };
            if pending && verification == ValidatorVerification::Verified {
                budget.record_verified(validator_address, peer_id, timestamp);
            }
            verifications.push((
                *contact.peer_id(),
                contact.contact().timestamp(),
                verification,
            ));
            if verification.is_node_wide_unverifiable() {
                debug!(
                    ?verification,
                    "Cannot verify validator claims right now, skipping checks until next tick",
                );
                budget.exhaust();
                break;
            }
        }
        verifications
    }

    /// Adds addresses into our own contact within the peer contact book
    pub fn add_own_addresses(&self, addresses: Vec<Multiaddr>) {
        self.peer_contact_book
            .write()
            .add_own_addresses(addresses, &self.keypair)
    }

    /// Returns whether an address in `Multiaddr` format is a dialable websocket address
    pub fn is_address_dialable(&self, address: &Multiaddr) -> bool {
        self.peer_contact_book.read().is_address_dialable(address)
    }

    /// Returns a reference to the peer contact book
    fn peer_contact_book(&self) -> Arc<RwLock<PeerContactBook>> {
        Arc::clone(&self.peer_contact_book)
    }
}

impl NetworkBehaviour for Behaviour {
    type ConnectionHandler = Handler;
    type ToSwarm = Event;

    fn handle_established_inbound_connection(
        &mut self,
        _connection_id: ConnectionId,
        peer: PeerId,
        _local_addr: &Multiaddr,
        remote_addr: &Multiaddr,
    ) -> Result<Handler, ConnectionDenied> {
        Ok(Handler::new(
            peer,
            self.config.clone(),
            self.keypair.clone(),
            self.peer_contact_book(),
            Arc::clone(&self.validator_verifier),
            Arc::clone(&self.validator_claim_budget),
            remote_addr.clone(),
        ))
    }

    fn handle_established_outbound_connection(
        &mut self,
        _connection_id: ConnectionId,
        peer: PeerId,
        addr: &Multiaddr,
        _role_override: Endpoint,
        _port_use: PortUse,
    ) -> Result<Handler, ConnectionDenied> {
        Ok(Handler::new(
            peer,
            self.config.clone(),
            self.keypair.clone(),
            self.peer_contact_book(),
            Arc::clone(&self.validator_verifier),
            Arc::clone(&self.validator_claim_budget),
            addr.clone(),
        ))
    }

    fn handle_pending_outbound_connection(
        &mut self,
        _connection_id: ConnectionId,
        maybe_peer: Option<PeerId>,
        _addresses: &[Multiaddr],
        _effective_role: Endpoint,
    ) -> Result<Vec<Multiaddr>, ConnectionDenied> {
        let peer_id = match maybe_peer {
            None => return Ok(vec![]),
            Some(peer) => peer,
        };

        Ok(self
            .peer_contact_book
            .read()
            .get_addresses(&peer_id)
            .unwrap_or_default())
    }

    fn poll(&mut self, cx: &mut Context) -> Poll<DiscoveryToSwarm> {
        // Emit events
        if let Some(event) = self.events.pop_front() {
            return Poll::Ready(event);
        }

        // Poll house-keeping timer
        match self.house_keeping_timer.poll_next_unpin(cx) {
            Poll::Ready(Some(_)) => {
                trace!("Doing house-keeping in peer address book");

                // Refill the budget that bounds how many validator claims arriving via
                // handshakes and peer-address updates get verified before the next tick.
                self.validator_claim_budget.reset();

                // Re-check the claims we could not verify before, for example because the
                // staking contract was still incomplete when the contact arrived. Checking reads
                // blockchain state, so take a snapshot, check outside the lock, and apply the
                // results afterwards. If this finds that this node still cannot verify any
                // claim, it empties the budget we just refilled again.
                //
                // Verified bindings that stopped being refreshed are re-checked as well, after the
                // pending claims, so that a binding whose claim no longer holds, e.g. because the
                // validator rotated its signing key, does not outlive the check that finds out.
                let (unverified, stale_bindings) = {
                    let book = self.peer_contact_book.read();
                    (
                        book.unverified_validator_contacts(),
                        book.stale_verified_validator_contacts(),
                    )
                };
                let mut recheck_window = self.select_recheck_window(&unverified);
                let stale_window = self.select_stale_binding_window(&stale_bindings);
                // A pending refresh of a stale binding may be in both windows.
                for contact in &stale_window {
                    if !recheck_window
                        .iter()
                        .any(|picked| picked.peer_id() == contact.peer_id())
                    {
                        recheck_window.push(Arc::clone(contact));
                    }
                }
                let verifications = Self::recheck_validator_claims(
                    &recheck_window,
                    &*self.validator_verifier,
                    &self.validator_claim_budget,
                );
                // Only a stale binding whose claim was actually checked waits for its next
                // re-check. One that was skipped, e.g. because the sweep stopped early, stays
                // first in line.
                if let Ok(unix_time) = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
                    for contact in &stale_window {
                        if verifications
                            .iter()
                            .any(|(peer_id, _, _)| peer_id == contact.peer_id())
                        {
                            contact.mark_binding_rechecked(unix_time);
                        }
                    }
                }

                let mut peer_address_book = self.peer_contact_book.write();
                peer_address_book.update_own_contact(&self.keypair);
                peer_address_book.house_keeping();
                peer_address_book.apply_validator_verifications(verifications);
            }
            Poll::Ready(None) => unreachable!(),
            Poll::Pending => {}
        }

        Poll::Pending
    }

    fn on_swarm_event(&mut self, event: FromSwarm) {
        match event {
            FromSwarm::ConnectionClosed(ConnectionClosed {
                peer_id,
                remaining_established,
                ..
            }) => {
                if remaining_established == 0 {
                    // There are no more remaining connections to this peer
                    self.connected_peers.remove(&peer_id);
                }
            }
            FromSwarm::ConnectionEstablished(ConnectionEstablished {
                peer_id,
                other_established: 0, // This is the first connection to this peer
                ..
            }) => {
                self.connected_peers.insert(peer_id);
            }
            _ => {}
        }
    }

    fn on_connection_handler_event(
        &mut self,
        peer_id: PeerId,
        _connection: ConnectionId,
        event: HandlerOutEvent,
    ) {
        match event {
            HandlerOutEvent::PeerExchangeEstablished {
                peer_address,
                peer_contact: signed_peer_contact,
            } => {
                if let Some(peer_contact) = self.peer_contact_book.read().get(&peer_id) {
                    self.events
                        .push_back(ToSwarm::GenerateEvent(Event::Established {
                            peer_id: signed_peer_contact.public_key().clone().to_peer_id(),
                            peer_address,
                            peer_contact: peer_contact.contact().clone(),
                        }));
                }
            }
            HandlerOutEvent::ObservedAddress { observed_address } => {
                self.events
                    .push_back(ToSwarm::NewExternalAddrCandidate(observed_address));
            }
            HandlerOutEvent::Update => self.events.push_back(ToSwarm::GenerateEvent(Event::Update)),
            HandlerOutEvent::Error(_) => self.events.push_back(ToSwarm::CloseConnection {
                peer_id,
                connection: CloseConnection::All,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashSet,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
        task::Context,
        time::SystemTime,
    };

    use futures::task::noop_waker_ref;
    use libp2p::{identity::Keypair, swarm::NetworkBehaviour, PeerId};
    use nimiq_hash::Blake2bHash;
    use nimiq_keys::{Address, KeyPair, SecureGenerate};
    use nimiq_network_interface::{peer_info::Services, validator_claim::ValidatorClaimSigner};
    use nimiq_test_log::test;
    use nimiq_test_utils::test_rng;
    use parking_lot::RwLock;

    use super::{Behaviour, Config};
    use crate::discovery::{
        peer_contacts::{
            CheckedPeerContact, PeerContact, PeerContactBook, PeerContactInfo, SignedPeerContact,
            ValidatorInfo,
        },
        validator_verifier::{
            InvalidReason, SignedValidatorClaim, UnverifiableReason, ValidatorClaimBudget,
            ValidatorClaimVerifier, ValidatorVerification,
        },
    };

    /// A verifier that answers the claims it is asked about with `outcomes`, in order, and counts
    /// how often it was asked, i.e. how many blockchain reads a real verifier would have done.
    struct ScriptedVerifier {
        outcomes: Vec<ValidatorVerification>,
        calls: AtomicUsize,
    }

    impl ScriptedVerifier {
        fn new(outcomes: Vec<ValidatorVerification>) -> Self {
            Self {
                outcomes,
                calls: AtomicUsize::new(0),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::Relaxed)
        }
    }

    impl ValidatorClaimVerifier for ScriptedVerifier {
        fn verify_validator_claim(
            &self,
            _signed_claim: &SignedValidatorClaim,
        ) -> ValidatorVerification {
            let call = self.calls.fetch_add(1, Ordering::Relaxed);
            *self
                .outcomes
                .get(call)
                .expect("the verifier was asked about more claims than scripted")
        }
    }

    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    /// A contact of the peer of `keypair` at `timestamp`, carrying a validator claim if
    /// `with_claim`. The scripted verifier never looks at the claim, so any validator will do.
    fn signed_contact(keypair: &Keypair, timestamp: u64, with_claim: bool) -> SignedPeerContact {
        let mut contact = PeerContact::new(
            ["/ip4/127.0.0.1/tcp/8443".parse().unwrap()],
            keypair.public(),
            Services::all(),
            timestamp,
        )
        .unwrap();
        if with_claim {
            let signer = ValidatorClaimSigner::new(
                Address::from([1u8; 20]),
                KeyPair::generate(&mut test_rng(false)),
            );
            let signed_claim = signer.sign(contact.peer_id(), contact.timestamp());
            contact.set_validator_info(Some(ValidatorInfo::new(
                signer.validator_address().clone(),
                signed_claim.signature,
            )));
        }
        contact.sign(keypair)
    }

    /// A stored contact of a fresh peer, carrying a validator claim if `with_claim`.
    fn stored_contact(with_claim: bool) -> Arc<PeerContactInfo> {
        let keypair = Keypair::generate_ed25519();
        Arc::new(PeerContactInfo::from(signed_contact(
            &keypair, 1_000, with_claim,
        )))
    }

    /// A behaviour checking validator claims with `verifier`, whose contact book holds a contact
    /// of a fresh peer with a claim that was not checked yet. Returns the behaviour along with its
    /// contact book and the peer ID of that contact.
    fn behaviour_with_pending_claim(
        verifier: Arc<dyn ValidatorClaimVerifier>,
    ) -> (Behaviour, Arc<RwLock<PeerContactBook>>, PeerId) {
        let keypair = Keypair::generate_ed25519();
        let book = Arc::new(RwLock::new(PeerContactBook::new(
            signed_contact(&keypair, now_secs(), false),
            false,
            true,
            true,
        )));

        let pending = signed_contact(&Keypair::generate_ed25519(), now_secs(), true);
        let peer_id = pending.peer_id();
        book.write().insert(CheckedPeerContact::pending(pending));
        assert_eq!(book.read().unverified_validator_contacts().len(), 1);

        let behaviour = Behaviour::new(
            Config::new(Blake2bHash::default(), Services::all(), false),
            keypair,
            Arc::clone(&book),
            verifier,
        );
        (behaviour, book, peer_id)
    }

    /// Lets the house-keeping timer of `behaviour` fire once, then polls the behaviour until it
    /// has handled that tick. Needs a runtime whose time is paused, so that waiting for the timer
    /// takes no real time.
    async fn run_house_keeping_tick(behaviour: &mut Behaviour) {
        nimiq_time::sleep(behaviour.config.house_keeping_interval).await;
        let mut cx = Context::from_waker(noop_waker_ref());
        // Every call first emits a queued event, if there is one. The first call that finds none
        // handles the tick and returns pending.
        while behaviour.poll(&mut cx).is_ready() {}
    }

    /// What [`Behaviour::recheck_validator_claims`] returns for `contact` checked as `outcome`.
    fn result(
        contact: &PeerContactInfo,
        outcome: ValidatorVerification,
    ) -> (PeerId, u64, ValidatorVerification) {
        (*contact.peer_id(), contact.contact().timestamp(), outcome)
    }

    /// Simulates `MAX_RECHECKED_CLAIMS_PER_TICK` (128) worth of selection ticks over the given
    /// backlog size, returning the set of indices (into a hypothetical peer-ID-ordered pool of
    /// that size) that were ever selected.
    fn simulate_ticks(len: usize, ticks: usize) -> HashSet<usize> {
        let mut cursor = 0;
        let mut seen = HashSet::new();
        for _ in 0..ticks {
            let (start, window) = Behaviour::recheck_window_bounds(
                cursor,
                len,
                Behaviour::MAX_RECHECKED_CLAIMS_PER_TICK,
            );
            for offset in 0..window {
                seen.insert((start + offset) % len.max(1));
            }
            cursor = cursor.wrapping_add(Behaviour::MAX_RECHECKED_CLAIMS_PER_TICK);
        }
        seen
    }

    #[test]
    fn a_backlog_under_the_cap_is_fully_covered_in_one_tick() {
        let seen = simulate_ticks(50, 1);
        assert_eq!(seen, (0..50).collect());
    }

    #[test]
    fn a_backlog_over_the_cap_is_fully_covered_within_the_expected_number_of_ticks() {
        // A backlog well beyond what a single busy tick (128) can clear — e.g. a burst of
        // validator claims all becoming due for their first real check at once.
        let len: usize = 300;
        let expected_ticks = len.div_ceil(Behaviour::MAX_RECHECKED_CLAIMS_PER_TICK); // 3

        // Fewer ticks than needed must NOT reach full coverage: this is the regression this test
        // guards against — before the fix, `.take(N)` from an unrotated prefix would plateau here
        // forever instead of making progress.
        let partial = simulate_ticks(len, expected_ticks - 1);
        assert!(
            partial.len() < len,
            "coverage must still be partial before enough ticks have run"
        );

        let full = simulate_ticks(len, expected_ticks);
        assert_eq!(
            full,
            (0..len).collect(),
            "every index must be reached within the expected tick count"
        );
    }

    #[test]
    fn the_cursor_keeps_advancing_and_wrapping_coverage_stays_fair_as_the_backlog_shrinks() {
        // The backlog shrinking between ticks (contacts resolved, evicted, or replaced) must not
        // strand the cursor out of range or break coverage of what remains.
        let mut cursor = Behaviour::MAX_RECHECKED_CLAIMS_PER_TICK * 10; // far beyond a small len
        let len = 40;
        let (start, window) =
            Behaviour::recheck_window_bounds(cursor, len, Behaviour::MAX_RECHECKED_CLAIMS_PER_TICK);
        assert!(start < len);
        assert_eq!(window, len); // the whole (small) backlog fits in one window
        cursor = cursor.wrapping_add(Behaviour::MAX_RECHECKED_CLAIMS_PER_TICK);
        let (start, _) =
            Behaviour::recheck_window_bounds(cursor, len, Behaviour::MAX_RECHECKED_CLAIMS_PER_TICK);
        assert!(start < len);
    }

    #[test]
    fn an_empty_backlog_selects_nothing_without_panicking() {
        let (start, window) =
            Behaviour::recheck_window_bounds(12345, 0, Behaviour::MAX_RECHECKED_CLAIMS_PER_TICK);
        assert_eq!((start, window), (0, 0));
    }

    #[test]
    fn the_recheck_sweep_stops_at_the_first_node_wide_unverifiable_claim() {
        let window: Vec<_> = (0..5).map(|_| stored_contact(true)).collect();
        let verifier = ScriptedVerifier::new(vec![
            ValidatorVerification::Verified,
            ValidatorVerification::Invalid(InvalidReason::InvalidSignature),
            ValidatorVerification::Unverifiable(UnverifiableReason::StateIncomplete),
        ]);
        let budget = ValidatorClaimBudget::with_refresh_reserve(5, 5);

        let results = Behaviour::recheck_validator_claims(&window, &verifier, &budget);

        // The claims after the node-wide `Unverifiable` are not checked at all, the outcomes
        // computed up to it are still returned, and the discovery connections are stopped from
        // checking inbound claims until the next tick as well.
        assert_eq!(verifier.calls(), 3);
        assert_eq!(
            results,
            vec![
                result(&window[0], ValidatorVerification::Verified),
                result(
                    &window[1],
                    ValidatorVerification::Invalid(InvalidReason::InvalidSignature)
                ),
                result(
                    &window[2],
                    ValidatorVerification::Unverifiable(UnverifiableReason::StateIncomplete)
                ),
            ]
        );
        assert_eq!(budget.remaining(), (0, 0));
    }

    #[test]
    fn the_recheck_sweep_checks_the_whole_window_while_claims_can_be_verified() {
        let window = vec![
            stored_contact(true),
            stored_contact(false),
            stored_contact(true),
            stored_contact(true),
        ];
        let verifier = ScriptedVerifier::new(vec![
            ValidatorVerification::Invalid(InvalidReason::UnknownValidator),
            ValidatorVerification::Verified,
            ValidatorVerification::Invalid(InvalidReason::InvalidSignature),
        ]);
        let budget = ValidatorClaimBudget::new(1);

        let results = Behaviour::recheck_validator_claims(&window, &verifier, &budget);

        // A contact without a claim is skipped without asking the verifier, and conclusive
        // outcomes leave the budget alone.
        assert_eq!(verifier.calls(), 3);
        assert_eq!(
            results,
            vec![
                result(
                    &window[0],
                    ValidatorVerification::Invalid(InvalidReason::UnknownValidator)
                ),
                result(&window[2], ValidatorVerification::Verified),
                result(
                    &window[3],
                    ValidatorVerification::Invalid(InvalidReason::InvalidSignature)
                ),
            ]
        );
        assert_eq!(budget.remaining(), (1, 0));
    }

    #[test(tokio::test)]
    async fn the_shared_claim_budget_has_the_refresh_reserve() {
        let (behaviour, _book, _peer_id) =
            behaviour_with_pending_claim(Arc::new(ScriptedVerifier::new(vec![])));

        assert_eq!(
            behaviour.validator_claim_budget.remaining(),
            (
                Behaviour::MAX_INBOUND_CLAIMS_VERIFIED_PER_TICK,
                Behaviour::INBOUND_REFRESH_CLAIM_RESERVE_PER_TICK,
            )
        );
    }

    #[test(tokio::test(start_paused = true))]
    async fn a_house_keeping_tick_refills_the_shared_budget() {
        let verifier = Arc::new(ScriptedVerifier::new(vec![ValidatorVerification::Verified]));
        let (mut behaviour, book, peer_id) = behaviour_with_pending_claim(verifier.clone());
        behaviour.validator_claim_budget.exhaust();

        run_house_keeping_tick(&mut behaviour).await;

        // The claim was re-checked...
        assert_eq!(verifier.calls(), 1);
        assert!(book.read().get(&peer_id).unwrap().is_validator_verified());

        // ...and the discovery connections can check inbound claims again, refreshes included.
        assert_eq!(
            behaviour.validator_claim_budget.remaining(),
            (
                Behaviour::MAX_INBOUND_CLAIMS_VERIFIED_PER_TICK,
                Behaviour::INBOUND_REFRESH_CLAIM_RESERVE_PER_TICK,
            )
        );
    }

    #[test(tokio::test(start_paused = true))]
    async fn a_house_keeping_tick_that_cannot_verify_claims_exhausts_the_shared_budget() {
        let verifier = Arc::new(ScriptedVerifier::new(vec![
            ValidatorVerification::Unverifiable(UnverifiableReason::StateIncomplete),
        ]));
        let (mut behaviour, book, peer_id) = behaviour_with_pending_claim(verifier.clone());

        run_house_keeping_tick(&mut behaviour).await;

        // The re-check found that this node cannot verify any claim yet, so the budget it had just
        // refilled for the discovery connections is empty again until the next tick.
        assert_eq!(verifier.calls(), 1);
        assert!(book
            .read()
            .get(&peer_id)
            .unwrap()
            .validator_claim_needs_recheck());
        assert_eq!(behaviour.validator_claim_budget.remaining(), (0, 0));
    }
    #[test]
    fn the_recheck_sweep_checks_only_a_validators_share_of_claims() {
        // Pending claims to the same validator.
        let mut book = PeerContactBook::new(
            signed_contact(&Keypair::generate_ed25519(), now_secs(), false),
            false,
            true,
            true,
        );
        for _ in 0..5 {
            book.insert(CheckedPeerContact::pending(signed_contact(
                &Keypair::generate_ed25519(),
                now_secs(),
                true,
            )));
        }
        let window = book.unverified_validator_contacts();
        assert_eq!(window.len(), 5);
        let verifier = ScriptedVerifier::new(vec![ValidatorVerification::Verified; 5]);
        let budget = ValidatorClaimBudget::with_refresh_reserve(5, 5).with_per_validator_limit(2);

        let results = Behaviour::recheck_validator_claims(&window, &verifier, &budget);

        assert_eq!(verifier.calls(), 2);
        assert_eq!(
            results,
            vec![
                result(&window[0], ValidatorVerification::Verified),
                result(&window[1], ValidatorVerification::Verified),
            ]
        );
    }

    #[test(tokio::test(start_paused = true))]
    async fn a_house_keeping_tick_ends_a_stale_binding_whose_claim_no_longer_holds() {
        let keypair = Keypair::generate_ed25519();
        let book = Arc::new(RwLock::new(PeerContactBook::new(
            signed_contact(&keypair, now_secs(), false),
            false,
            true,
            true,
        )));

        // A binding verified in a contact that was not refreshed for a while.
        let stale = signed_contact(
            &Keypair::generate_ed25519(),
            now_secs() - PeerContactBook::STALE_BINDING_AGE - 10,
            true,
        );
        let peer_id = stale.peer_id();
        book.write().insert(CheckedPeerContact::check(
            stale,
            &ScriptedVerifier::new(vec![ValidatorVerification::Verified]),
        ));
        let address = Address::from([1u8; 20]);
        assert_eq!(book.read().get_validator_peer_ids(&address), vec![peer_id]);

        // Meanwhile the validator rotated its signing key, so its claim no longer holds.
        let verifier = Arc::new(ScriptedVerifier::new(vec![ValidatorVerification::Invalid(
            InvalidReason::InvalidSignature,
        )]));
        let mut behaviour = Behaviour::new(
            Config::new(Blake2bHash::default(), Services::all(), false),
            keypair,
            Arc::clone(&book),
            verifier.clone(),
        );

        run_house_keeping_tick(&mut behaviour).await;

        assert_eq!(verifier.calls(), 1);
        assert!(book.read().get_validator_peer_ids(&address).is_empty());
        assert!(!book.read().get(&peer_id).unwrap().is_gossipable());
    }

    // A tick that stops before reaching a stale binding, because this node cannot verify any claim
    // right now, must not count as its re-check, or the binding would wait another
    // `STALE_BINDING_AGE` before a check finds out that its claim no longer holds.
    #[test(tokio::test(start_paused = true))]
    async fn a_stale_binding_the_sweep_did_not_reach_is_rechecked_on_the_next_tick() {
        let keypair = Keypair::generate_ed25519();
        let book = Arc::new(RwLock::new(PeerContactBook::new(
            signed_contact(&keypair, now_secs(), false),
            false,
            true,
            true,
        )));

        let stale = signed_contact(
            &Keypair::generate_ed25519(),
            now_secs() - PeerContactBook::STALE_BINDING_AGE - 10,
            true,
        );
        let peer_id = stale.peer_id();
        book.write().insert(CheckedPeerContact::check(
            stale,
            &ScriptedVerifier::new(vec![ValidatorVerification::Verified]),
        ));
        let address = Address::from([1u8; 20]);
        // A pending claim of another validator, which is checked before the stale binding.
        pending_claim(&mut book.write(), 2);

        let verifier = Arc::new(ScriptedVerifier::new(vec![
            // First tick: the pending claim shows that no claim can be verified right now.
            ValidatorVerification::Unverifiable(UnverifiableReason::StateIncomplete),
            // Second tick: the pending claim, then the stale binding, whose claim no longer holds.
            ValidatorVerification::Verified,
            ValidatorVerification::Invalid(InvalidReason::InvalidSignature),
        ]));
        let mut behaviour = Behaviour::new(
            Config::new(Blake2bHash::default(), Services::all(), false),
            keypair,
            Arc::clone(&book),
            verifier.clone(),
        );

        run_house_keeping_tick(&mut behaviour).await;
        assert_eq!(verifier.calls(), 1);
        assert_eq!(book.read().get_validator_peer_ids(&address), vec![peer_id]);

        run_house_keeping_tick(&mut behaviour).await;
        assert_eq!(verifier.calls(), 3);
        assert!(book.read().get_validator_peer_ids(&address).is_empty());
    }

    /// A pending claim to the validator at `address_seed` of a fresh peer, stored in `book`.
    fn pending_claim(book: &mut PeerContactBook, address_seed: u8) {
        let keypair = Keypair::generate_ed25519();
        let mut contact = PeerContact::new(
            ["/ip4/127.0.0.1/tcp/8443".parse().unwrap()],
            keypair.public(),
            Services::all(),
            now_secs(),
        )
        .unwrap();
        let signer = ValidatorClaimSigner::new(
            Address::from([address_seed; 20]),
            KeyPair::generate(&mut test_rng(false)),
        );
        let signed_claim = signer.sign(contact.peer_id(), contact.timestamp());
        contact.set_validator_info(Some(ValidatorInfo::new(
            signer.validator_address().clone(),
            signed_claim.signature,
        )));
        book.insert(CheckedPeerContact::pending(contact.sign(&keypair)));
    }

    #[test]
    fn the_recheck_window_takes_only_a_validators_share_of_its_claims() {
        let mut book = PeerContactBook::new(
            signed_contact(&Keypair::generate_ed25519(), now_secs(), false),
            false,
            true,
            true,
        );
        for _ in 0..10 {
            pending_claim(&mut book, 1);
        }
        for _ in 0..3 {
            pending_claim(&mut book, 2);
        }
        let pool = book.unverified_validator_contacts();

        let mut cursor = 0;
        let window = Behaviour::select_window(&mut cursor, &pool, 128, 4);

        let count = |seed: u8| {
            window
                .iter()
                .filter(|contact| contact.validator_address() == Some(&Address::from([seed; 20])))
                .count()
        };
        assert_eq!((count(1), count(2)), (4, 3));

        // The window was not full, so the next one starts with the first claim this one skipped,
        // and the windows take turns with the claims of validator 1.
        let mut picked = HashSet::new();
        let mut cursor = 0;
        for _ in 0..3 {
            let window = Behaviour::select_window(&mut cursor, &pool, 128, 4);
            assert_eq!(window.len(), 7);
            picked.extend(window.iter().map(|contact| *contact.peer_id()));
        }
        assert_eq!(picked.len(), pool.len());

        // A full window stops looking, and the next one continues after it.
        let mut cursor = 0;
        let window = Behaviour::select_window(&mut cursor, &pool, 2, usize::MAX);
        assert_eq!(window.len(), 2);
        assert_eq!(cursor, 2);
    }

    #[test]
    fn a_stale_binding_recheck_does_not_use_up_the_validators_share() {
        let mut book = PeerContactBook::new(
            signed_contact(&Keypair::generate_ed25519(), now_secs(), false),
            false,
            true,
            true,
        );
        pending_claim(&mut book, 1);
        // A contact verified before, as the stale window holds them, followed by a pending one.
        let window = vec![
            stored_verified_contact(),
            book.unverified_validator_contacts()[0].clone(),
        ];
        let verifier = ScriptedVerifier::new(vec![ValidatorVerification::Verified; 2]);
        let budget = ValidatorClaimBudget::with_refresh_reserve(5, 5).with_per_validator_limit(1);

        let results = Behaviour::recheck_validator_claims(&window, &verifier, &budget);

        assert_eq!(verifier.calls(), 2);
        assert_eq!(results.len(), 2);
    }

    /// A contact of a fresh peer claiming the validator at `[1; 20]`, as if its claim had been
    /// verified.
    fn stored_verified_contact() -> Arc<PeerContactInfo> {
        let contact = stored_contact(true);
        contact.set_validator_verified(true);
        contact
    }
    #[test]
    fn only_verified_claims_count_towards_a_validators_share_in_the_sweep() {
        let mut book = PeerContactBook::new(
            signed_contact(&Keypair::generate_ed25519(), now_secs(), false),
            false,
            true,
            true,
        );
        pending_claim(&mut book, 1);
        pending_claim(&mut book, 1);
        let window = book.unverified_validator_contacts();
        // The first one turns out to be bogus, which must not use up the share of the validator
        // it names.
        let verifier = ScriptedVerifier::new(vec![
            ValidatorVerification::Invalid(InvalidReason::InvalidSignature),
            ValidatorVerification::Verified,
        ]);
        let budget = ValidatorClaimBudget::with_refresh_reserve(5, 5).with_per_validator_limit(1);

        let results = Behaviour::recheck_validator_claims(&window, &verifier, &budget);

        assert_eq!(verifier.calls(), 2);
        assert_eq!(results.len(), 2);
    }

    #[test(tokio::test)]
    async fn the_shared_claim_budget_limits_each_validator() {
        let (behaviour, _book, _peer_id) =
            behaviour_with_pending_claim(Arc::new(ScriptedVerifier::new(vec![])));
        let budget = &behaviour.validator_claim_budget;
        let address = Address::from([7u8; 20]);

        for timestamp in 0..Behaviour::VERIFIED_CLAIMS_PER_VALIDATOR_PER_TICK as u64 {
            assert!(budget.may_verify(&address, &PeerId::random(), timestamp));
            budget.record_verified(&address, PeerId::random(), timestamp);
        }
        assert!(!budget.may_verify(&address, &PeerId::random(), 0));
    }

    /// The limits that keep a single validator from spending the claim budget of every node are
    /// meant to be small: an honest validator binds one peer, and refreshes its contact once per
    /// tick.
    #[test]
    fn the_limits_on_a_single_validator_are_small() {
        assert_eq!(PeerContactBook::MAX_BINDINGS_PER_VALIDATOR, 4);
        assert_eq!(Behaviour::VERIFIED_CLAIMS_PER_VALIDATOR_PER_TICK, 4);
    }
}
