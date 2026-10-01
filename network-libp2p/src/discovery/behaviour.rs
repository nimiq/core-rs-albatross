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

    /// The peer ID of the last contact the re-check sweep is done with in the peer-ID-ordered
    /// list returned by [`PeerContactBook::unverified_validator_contacts`], if any. The next sweep
    /// continues after it (see [`Self::recheck_pending_claims`]).
    ///
    /// A backlog of contacts needing a re-check can persist across many ticks (e.g. right after
    /// this node's staking-contract view becomes complete, when everything that was
    /// `Unverifiable(StateIncomplete)` becomes due for its first real check at once). Always
    /// starting at the same contact every tick would let claims that keep joining the list ahead
    /// of a contact keep it from being reached, delaying both promotion of valid claims and
    /// revocation of previous verified bindings. Continuing after the last contact the sweep is
    /// done with, a contact is reached once the claims of the contacts between the cursor and it
    /// are checked or passed over. Remembering a peer ID rather than a position keeps contacts that
    /// leave or join the list in between from moving the others.
    validator_recheck_cursor: Option<PeerId>,

    /// Rotating offset into the peer-ID-ordered list returned by
    /// [`PeerContactBook::stale_verified_validator_contacts`] (see
    /// [`Self::select_stale_binding_window`]).
    stale_binding_recheck_cursor: usize,
}

/// What [`Behaviour::recheck_pending_claims`] did in one tick.
struct PendingRecheck<'a> {
    /// The pool it went through, ordered by peer ID.
    pool: &'a [Arc<PeerContactInfo>],
    /// The outcomes of the claims it checked, see [`Behaviour::recheck_validator_claims`].
    verifications: Vec<(PeerId, u64, ValidatorVerification)>,
    /// Where in `pool` it started.
    start: usize,
    /// How many contacts of `pool` it looked at from `start` on, wrapping around.
    looked_at: usize,
}

impl PendingRecheck<'_> {
    /// Whether it stopped at a claim that cannot be verified for a node-wide reason.
    fn stopped(&self) -> bool {
        self.verifications
            .last()
            .is_some_and(|(_, _, verification)| verification.is_node_wide_unverifiable())
    }

    /// Whether it looked at the contact of `peer_id`, i.e. checked its claim or passed it over.
    fn has_looked_at(&self, peer_id: &PeerId) -> bool {
        let len = self.pool.len();
        self.pool
            .binary_search_by_key(peer_id, |contact| *contact.peer_id())
            .is_ok_and(|index| (index + len - self.start) % len < self.looked_at)
    }
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

    /// Number of contacts claiming the same validator address that can verify per tick when they
    /// arrive: the first contact of each peer, and each contact that is fresh (see
    /// [`Self::FRESH_VALIDATOR_CONTACT_AGE`]) and supersedes an older one of the same peer that
    /// did (see [`ValidatorClaimBudget::may_verify`]). The re-check sweep verifies at most as many
    /// pending claims to the same validator per tick on top of that (see
    /// [`Self::recheck_validator_claims`]).
    ///
    /// An honest validator re-signs its contact once per tick, and binds a new peer only when it
    /// starts or moves to another node, so it needs one or two, or three when an older contact of
    /// its node gets here first in a tick. Re-signing its contact outside that schedule, e.g. when
    /// its node restarts or installs its claim signer, can take one more, and so can the short
    /// overlap in which two of its regularly re-signed contacts are fresh (see
    /// [`ValidatorClaimBudget`]). A contact beyond the share is left pending, and verified by the
    /// next re-check sweep. A validator can sign claims for as many peer IDs as it likes, though,
    /// and refresh each of them every second. Its verified contacts are gossiped, so without this
    /// limit a single one could spend the general budget of every node, keeping the first claims of
    /// other validators from being verified.
    const VERIFIED_CLAIMS_PER_VALIDATOR_PER_TICK: usize = 4;

    /// How young a contact must be, on this node's clock, to verify on arrival after an older
    /// contact of the same peer did in the same tick (see
    /// [`ValidatorClaimBudget::with_fresh_contact_age`]).
    ///
    /// Peers re-sign their contact every house-keeping tick (60 s by default), and no contact from
    /// the future is stored, so apart from a short overlap when a tick runs late (see
    /// [`ValidatorClaimBudget`]), at most one regularly re-signed contact of a peer is this young
    /// at a time, and a replayed older one never is. Contacts re-signed outside that schedule, e.g.
    /// when a node restarts or installs its claim signer, are this young as well. This follows the
    /// interval at which other nodes re-sign their contacts, not our own
    /// [`Config::house_keeping_interval`].
    const FRESH_VALIDATOR_CONTACT_AGE: Duration = Duration::from_secs(60);

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
                .with_per_validator_limit(Self::VERIFIED_CLAIMS_PER_VALIDATOR_PER_TICK)
                .with_fresh_contact_age(Self::FRESH_VALIDATOR_CONTACT_AGE),
            ),
            validator_recheck_cursor: None,
            stale_binding_recheck_cursor: 0,
        }
    }

    /// Pure windowing math behind [`Self::select_window`]: given a pool of `len` items
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

    /// Selects up to `MAX_RECHECKED_STALE_BINDINGS_PER_TICK` of the stale verified bindings
    /// returned by [`PeerContactBook::stale_verified_validator_contacts`] to re-check this tick,
    /// rotating `self.stale_binding_recheck_cursor` so that a backlog above the per-tick cap is
    /// not always cut off at the same place. Returns owned clones (an `Arc` bump each) rather
    /// than borrowing `pool`, so the mutable borrow of `self` this takes doesn't linger into the
    /// caller's subsequent use of `self.validator_verifier`.
    fn select_stale_binding_window(
        &mut self,
        pool: &[Arc<PeerContactInfo>],
    ) -> Vec<Arc<PeerContactInfo>> {
        Self::select_window(
            &mut self.stale_binding_recheck_cursor,
            pool,
            Self::MAX_RECHECKED_STALE_BINDINGS_PER_TICK,
        )
    }

    /// Takes up to `max_window` items of `pool`, starting at `cursor` (with wraparound), and
    /// advances `cursor` past them. See [`Self::recheck_window_bounds`].
    fn select_window(
        cursor: &mut usize,
        pool: &[Arc<PeerContactInfo>],
        max_window: usize,
    ) -> Vec<Arc<PeerContactInfo>> {
        let (start, len) = Self::recheck_window_bounds(*cursor, pool.len(), max_window);
        *cursor = cursor.wrapping_add(len);
        pool.iter().cycle().skip(start).take(len).cloned().collect()
    }

    /// Re-checks the pending claims of `pool`, which must be ordered by peer ID as
    /// [`PeerContactBook::unverified_validator_contacts`] returns it. It goes through them in
    /// that order, starting after `cursor` and wrapping around, until `max_checks` claims were
    /// checked or every contact was looked at, and moves `cursor` to the last contact it is done
    /// with. See [`Self::recheck_validator_claims`] for `verified_per_validator` and the outcomes.
    ///
    /// Only claims that are checked count towards `max_checks`: claims that cannot verify this tick
    /// because the validator they claim used up its share of verified claims are passed over
    /// without a check (see [`Self::recheck_validator_claims`]), and only claims that verify use
    /// up a share. Claims naming a validator can be made up without its key, and placed anywhere
    /// in `pool` by generating keys, but they cannot keep the validator's own claim from being
    /// checked any longer than as many claims to other validators would: they are checked one by
    /// one like any other claim. Keeping the sweep from reaching a claim for good still takes as
    /// many new claims ahead of it on every tick as the sweep checks, which holds up every claim
    /// behind them just the same. Getting such claims stored pending takes draining the budget
    /// that checks claims on arrival first, or using up the share of claims verified on arrival of
    /// the validator they name (see [`ValidatorClaimBudget::may_verify`]), since claims beyond it
    /// are stored pending without spending any budget. That takes a validator of one's own, or
    /// relaying enough genuine contacts of the named validator that are new to this node. This does
    /// not guard against either.
    ///
    /// If the sweep stops at a claim that cannot be verified for a node-wide reason, it is not
    /// done with that claim, and the next one starts with it.
    fn recheck_pending_claims<'a>(
        cursor: &mut Option<PeerId>,
        pool: &'a [Arc<PeerContactInfo>],
        max_checks: usize,
        verifier: &dyn ValidatorClaimVerifier,
        budget: &ValidatorClaimBudget,
        verified_per_validator: &mut HashMap<Address, usize>,
    ) -> PendingRecheck<'a> {
        let start = cursor.map_or(0, |last| {
            pool.partition_point(|contact| *contact.peer_id() <= last)
        }) % pool.len().max(1);
        let (verifications, looked_at) = Self::recheck_validator_claims(
            pool[start..].iter().chain(&pool[..start]),
            max_checks,
            verifier,
            budget,
            verified_per_validator,
        );
        let recheck = PendingRecheck {
            pool,
            verifications,
            start,
            looked_at,
        };

        let done = looked_at - usize::from(recheck.stopped());
        if let Some(last) = done.checked_sub(1) {
            *cursor = Some(*pool[(start + last) % pool.len()].peer_id());
        }
        recheck
    }

    /// Re-checks the validator claims of `contacts` in order, until `max_checks` claims were
    /// checked, returning the outcomes for [`PeerContactBook::apply_validator_verifications`]
    /// along with how many contacts it looked at.
    ///
    /// Checking reads blockchain state, so this must be called without holding the contact book
    /// lock. It stops at the first claim that cannot be verified for a node-wide reason (see
    /// [`UnverifiableReason::is_node_wide`](super::validator_verifier::UnverifiableReason::is_node_wide)),
    /// e.g. while this node is still syncing its staking contract: every other claim would come
    /// back the same way, each at the cost of another blockchain read on the swarm task. It then
    /// also exhausts `budget`, so that the discovery connections stop checking inbound claims
    /// until the next tick as well. The outcomes computed up to that point are still returned.
    ///
    /// A pending claim is only checked while it could verify this tick: neither the share of
    /// claims that verified on arrival (see [`ValidatorClaimBudget::may_verify`]) nor the sweep's
    /// own share in `verified_per_validator`, which the caller keeps across both passes of a tick,
    /// rules it out. Otherwise it is passed over. The pending claims the sweep verifies count
    /// against its own share only, and are not recorded in `budget`. Once a contact of a peer
    /// verified there, a newer one of the same peer only verifies on arrival while it is fresh,
    /// so the peer's next refresh, which arrives later in the same tick, would otherwise be left
    /// pending whenever it reaches us late. It would replace the contact the sweep just verified,
    /// which would then not be gossiped until the next sweep, and the same could happen again on
    /// every tick.
    fn recheck_validator_claims<'a>(
        contacts: impl IntoIterator<Item = &'a Arc<PeerContactInfo>>,
        max_checks: usize,
        verifier: &dyn ValidatorClaimVerifier,
        budget: &ValidatorClaimBudget,
        verified_per_validator: &mut HashMap<Address, usize>,
    ) -> (Vec<(PeerId, u64, ValidatorVerification)>, usize) {
        let mut verifications = Vec::new();
        let mut looked_at = 0;
        let share = budget.verified_claims_per_validator();
        for contact in contacts {
            if verifications.len() == max_checks {
                break;
            }
            looked_at += 1;
            let Some(validator_address) = contact.validator_address() else {
                continue;
            };
            // A contact that was verified before only has its stale binding re-checked, which can
            // end the binding but never renew it (see
            // `PeerContactBook::apply_validator_verifications`), so it does not count towards the
            // validator's share.
            let (peer_id, timestamp) = (*contact.peer_id(), contact.contact().timestamp());
            let pending = contact.validator_claim_needs_recheck();
            // The share of claims that verified on arrival was just reset, but other discovery
            // connections may use it up, or verify a newer contact of this peer, while this runs.
            // The sweep checks the peer's stored, newest contact and records nothing, so it may
            // supersede an older one that verified on arrival.
            if pending
                && (verified_per_validator
                    .get(validator_address)
                    .is_some_and(|&verified| verified >= share)
                    || !budget.may_verify(validator_address, &peer_id, timestamp, true))
            {
                continue;
            }
            let Some(verification) = contact.signed().check_validator_claim(verifier) else {
                continue;
            };
            if pending && verification == ValidatorVerification::Verified {
                *verified_per_validator
                    .entry(validator_address.clone())
                    .or_default() += 1;
            }
            verifications.push((peer_id, timestamp, verification));
            if verification.is_node_wide_unverifiable() {
                debug!(
                    ?verification,
                    "Cannot verify validator claims right now, skipping checks until next tick",
                );
                budget.exhaust();
                break;
            }
        }
        (verifications, looked_at)
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
                let mut verified_per_validator = HashMap::new();
                let pending = Self::recheck_pending_claims(
                    &mut self.validator_recheck_cursor,
                    &unverified,
                    Self::MAX_RECHECKED_CLAIMS_PER_TICK,
                    &*self.validator_verifier,
                    &self.validator_claim_budget,
                    &mut verified_per_validator,
                );
                // Stale bindings are only picked if the sweep can go on, so that those it cannot
                // get to now are still first in line on the next tick. If the stale pass itself
                // stops because this node cannot verify any claim right now, the rest of its window
                // waits until the window comes around to it again.
                let stale_window = if pending.stopped() {
                    Vec::new()
                } else {
                    self.select_stale_binding_window(&stale_bindings)
                };
                // A pending refresh of a stale binding may be among the pending claims just looked
                // at.
                let (stale_verifications, _) = Self::recheck_validator_claims(
                    stale_window
                        .iter()
                        .filter(|contact| !pending.has_looked_at(contact.peer_id())),
                    usize::MAX,
                    &*self.validator_verifier,
                    &self.validator_claim_budget,
                    &mut verified_per_validator,
                );
                let mut verifications = pending.verifications;
                verifications.extend(stale_verifications);
                // Only a stale binding whose claim was actually checked, by either pass, waits for
                // its next re-check. One that was not, or whose check only found that this node
                // cannot verify any claim right now, is re-checked as soon as its turn comes again.
                if let Ok(unix_time) = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
                    for contact in &stale_window {
                        if verifications.iter().any(|(peer_id, _, verification)| {
                            peer_id == contact.peer_id()
                                && !verification.is_node_wide_unverifiable()
                        }) {
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
        collections::{HashMap, HashSet},
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

    use super::{Behaviour, Config, PendingRecheck};
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

    /// The outcome [`Behaviour::recheck_validator_claims`] returns for `contact` checked as
    /// `outcome`.
    fn result(
        contact: &PeerContactInfo,
        outcome: ValidatorVerification,
    ) -> (PeerId, u64, ValidatorVerification) {
        (*contact.peer_id(), contact.contact().timestamp(), outcome)
    }

    /// Simulates ticks of selecting up to `MAX_RECHECKED_STALE_BINDINGS_PER_TICK` (32) stale
    /// bindings from a list of the given length, which does not change in between, returning the
    /// set of indices into it that were ever selected.
    fn simulate_ticks(len: usize, ticks: usize) -> HashSet<usize> {
        let mut cursor = 0;
        let mut seen = HashSet::new();
        for _ in 0..ticks {
            let (start, window) = Behaviour::recheck_window_bounds(
                cursor,
                len,
                Behaviour::MAX_RECHECKED_STALE_BINDINGS_PER_TICK,
            );
            for offset in 0..window {
                seen.insert((start + offset) % len.max(1));
            }
            cursor = cursor.wrapping_add(window);
        }
        seen
    }

    #[test]
    fn stale_bindings_under_the_cap_are_all_selected_in_one_tick() {
        let seen = simulate_ticks(20, 1);
        assert_eq!(seen, (0..20).collect());
    }

    #[test]
    fn stale_bindings_over_the_cap_are_all_selected_within_the_expected_number_of_ticks() {
        let len: usize = 300;
        let expected_ticks = len.div_ceil(Behaviour::MAX_RECHECKED_STALE_BINDINGS_PER_TICK); // 10

        // Fewer ticks than needed must not reach full coverage, and taking the same prefix every
        // tick would never reach it.
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
    fn the_stale_binding_cursor_stays_in_range_as_the_list_shrinks() {
        // The list shrinking between ticks (bindings re-checked, refreshed or expired) must not
        // strand the cursor out of range.
        let max_window = Behaviour::MAX_RECHECKED_STALE_BINDINGS_PER_TICK;
        let mut cursor = max_window * 10; // far beyond a small len
        let len = 20;
        let (start, window) = Behaviour::recheck_window_bounds(cursor, len, max_window);
        assert!(start < len);
        assert_eq!(window, len); // the whole (small) list fits in one window
        cursor = cursor.wrapping_add(window);
        let (start, _) = Behaviour::recheck_window_bounds(cursor, len, max_window);
        assert!(start < len);
    }

    #[test]
    fn an_empty_list_selects_nothing_without_panicking() {
        let (start, window) = Behaviour::recheck_window_bounds(
            12345,
            0,
            Behaviour::MAX_RECHECKED_STALE_BINDINGS_PER_TICK,
        );
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

        let results = Behaviour::recheck_validator_claims(
            &window,
            usize::MAX,
            &verifier,
            &budget,
            &mut HashMap::new(),
        )
        .0;

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

        let results = Behaviour::recheck_validator_claims(
            &window,
            usize::MAX,
            &verifier,
            &budget,
            &mut HashMap::new(),
        )
        .0;

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

        let results = Behaviour::recheck_validator_claims(
            &window,
            usize::MAX,
            &verifier,
            &budget,
            &mut HashMap::new(),
        )
        .0;

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
        let verifier = Arc::new(ScriptedVerifier::new(vec![
            ValidatorVerification::Invalid(InvalidReason::InvalidSignature),
            ValidatorVerification::Invalid(InvalidReason::InvalidSignature),
        ]));
        let mut behaviour = Behaviour::new(
            Config::new(Blake2bHash::default(), Services::all(), false),
            keypair,
            Arc::clone(&book),
            verifier.clone(),
        );

        run_house_keeping_tick(&mut behaviour).await;

        // The binding ends right away. The claim verified before, so it is offered to the next
        // sweep for one more check...
        assert_eq!(verifier.calls(), 1);
        assert!(book.read().get_validator_peer_ids(&address).is_empty());
        let info = book.read().get(&peer_id).unwrap();
        assert!(!info.is_gossipable());
        assert!(info.validator_claim_needs_recheck());
        assert!(book.read().stale_verified_validator_contacts().is_empty());

        // ...which settles it.
        run_house_keeping_tick(&mut behaviour).await;
        assert_eq!(verifier.calls(), 2);
        assert!(book.read().get_validator_peer_ids(&address).is_empty());
        assert!(!info.is_gossipable());
        assert!(!info.validator_claim_needs_recheck());

        // After that, nothing is left to check.
        run_house_keeping_tick(&mut behaviour).await;
        assert_eq!(verifier.calls(), 2);
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
        // The stale bindings it did not get to are still first in line.
        assert_eq!(behaviour.stale_binding_recheck_cursor, 0);

        run_house_keeping_tick(&mut behaviour).await;
        assert_eq!(verifier.calls(), 3);
        assert!(book.read().get_validator_peer_ids(&address).is_empty());
    }

    // Once a contact of a peer verified on arrival, a newer one of the same peer only verifies on
    // arrival in the same tick if it is fresh. A pending claim of the peer that the sweep verified
    // must not count as such a contact, or the peer's next refresh, reaching us late in the same
    // tick, e.g. relayed, would be left pending and replace the verified contact, which would then
    // not be gossiped until the next sweep. That could repeat on every tick.
    #[test(tokio::test(start_paused = true))]
    async fn a_refresh_arriving_after_the_sweep_verified_its_peers_claim_is_verified_on_arrival() {
        let keypair = Keypair::generate_ed25519();
        let book = Arc::new(RwLock::new(PeerContactBook::new(
            signed_contact(&keypair, now_secs(), false),
            false,
            true,
            true,
        )));
        let peer_key = Keypair::generate_ed25519();
        let peer_id = peer_key.public().to_peer_id();
        let address = Address::from([1u8; 20]);
        let now = now_secs();
        book.write()
            .insert(CheckedPeerContact::pending(signed_contact(
                &peer_key,
                now - 300,
                true,
            )));
        let verifier = Arc::new(ScriptedVerifier::new(vec![
            ValidatorVerification::Verified;
            4
        ]));
        let mut behaviour = Behaviour::new(
            Config::new(Blake2bHash::default(), Services::all(), false),
            keypair,
            Arc::clone(&book),
            verifier.clone(),
        );

        // The sweep verifies the pending claim, and each refresh arriving after it is verified on
        // arrival, although none of them is fresh, so the contact stays gossipable.
        run_house_keeping_tick(&mut behaviour).await;
        assert_eq!(verifier.calls(), 1);
        for (tick, timestamp) in [now - 240, now - 180, now - 120].into_iter().enumerate() {
            assert!(!behaviour
                .validator_claim_budget
                .is_fresh(timestamp, now_secs()));
            if tick > 0 {
                run_house_keeping_tick(&mut behaviour).await;
            }
            let checked = CheckedPeerContact::check_new_with_budget(
                signed_contact(&peer_key, timestamp, true),
                &book,
                &*verifier,
                &behaviour.validator_claim_budget,
            );
            book.write().insert(checked);

            assert_eq!(verifier.calls(), 2 + tick);
            assert!(book.read().get(&peer_id).unwrap().is_gossipable());
            assert_eq!(book.read().get_validator_peer_ids(&address), vec![peer_id]);
        }
    }

    #[test]
    fn the_sweep_does_not_use_up_the_share_of_arriving_contacts() {
        let mut book = PeerContactBook::new(
            signed_contact(&Keypair::generate_ed25519(), now_secs(), false),
            false,
            true,
            true,
        );
        pending_claim(&mut book, 1);
        let pool = book.unverified_validator_contacts();
        let (peer_id, timestamp) = (*pool[0].peer_id(), pool[0].contact().timestamp());
        let verifier = ScriptedVerifier::new(vec![ValidatorVerification::Verified]);
        let budget = ValidatorClaimBudget::with_refresh_reserve(5, 5).with_per_validator_limit(1);

        let results = Behaviour::recheck_validator_claims(
            &pool,
            usize::MAX,
            &verifier,
            &budget,
            &mut HashMap::new(),
        )
        .0;
        assert_eq!(results.len(), 1);

        // Both a refresh of the same peer, even one that is not fresh, and another peer's first
        // claim may still verify on arrival.
        let address = Address::from([1u8; 20]);
        assert!(budget.may_verify(&address, &peer_id, timestamp + 1, false));
        assert!(budget.may_verify(&address, &PeerId::random(), timestamp, false));
    }

    /// A pending claim to the validator at `address_seed` of a fresh peer, stored in `book`.
    fn pending_claim(book: &mut PeerContactBook, address_seed: u8) {
        pending_claim_of(book, &Keypair::generate_ed25519(), address_seed);
    }

    /// A pending claim to the validator at `address_seed` of the peer of `keypair`, stored in
    /// `book`.
    fn pending_claim_of(book: &mut PeerContactBook, keypair: &Keypair, address_seed: u8) {
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
        book.insert(CheckedPeerContact::pending(contact.sign(keypair)));
    }

    /// Pending claims of fresh peers as [`PeerContactBook::unverified_validator_contacts`] returns
    /// them, i.e. ordered by peer ID, the one at each position claiming the validator at the
    /// address seed at that position of `address_seeds`.
    fn pending_pool(address_seeds: &[u8]) -> Vec<Arc<PeerContactInfo>> {
        let mut keypairs: Vec<_> = address_seeds
            .iter()
            .map(|_| Keypair::generate_ed25519())
            .collect();
        keypairs.sort_by_key(|keypair| keypair.public().to_peer_id());
        let mut book = PeerContactBook::new(
            signed_contact(&Keypair::generate_ed25519(), now_secs(), false),
            false,
            true,
            true,
        );
        for (keypair, &address_seed) in keypairs.iter().zip(address_seeds) {
            pending_claim_of(&mut book, keypair, address_seed);
        }
        let pool = book.unverified_validator_contacts();
        assert_eq!(pool.len(), address_seeds.len());
        assert!(pool.iter().zip(address_seeds).all(
            |(contact, &seed)| contact.validator_address() == Some(&Address::from([seed; 20]))
        ));
        pool
    }

    /// A verifier that verifies the claims of the peers in `valid`, rejects all others, and counts
    /// how often it was asked.
    #[derive(Default)]
    struct VerifiesOnly {
        valid: HashSet<PeerId>,
        calls: AtomicUsize,
    }

    impl VerifiesOnly {
        fn new(valid: impl IntoIterator<Item = PeerId>) -> Self {
            Self {
                valid: valid.into_iter().collect(),
                calls: AtomicUsize::new(0),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::Relaxed)
        }
    }

    impl ValidatorClaimVerifier for VerifiesOnly {
        fn verify_validator_claim(
            &self,
            signed_claim: &SignedValidatorClaim,
        ) -> ValidatorVerification {
            self.calls.fetch_add(1, Ordering::Relaxed);
            if self.valid.contains(&signed_claim.record.peer_id) {
                ValidatorVerification::Verified
            } else {
                ValidatorVerification::Invalid(InvalidReason::UnknownValidator)
            }
        }
    }

    /// Runs [`Behaviour::recheck_pending_claims`] as a tick of its own.
    fn recheck_pending<'a>(
        cursor: &mut Option<PeerId>,
        pool: &'a [Arc<PeerContactInfo>],
        max_checks: usize,
        verifier: &dyn ValidatorClaimVerifier,
        budget: &ValidatorClaimBudget,
    ) -> PendingRecheck<'a> {
        Behaviour::recheck_pending_claims(
            cursor,
            pool,
            max_checks,
            verifier,
            budget,
            &mut HashMap::new(),
        )
    }

    /// The peer IDs of the contacts `verifications` are about.
    fn checked_peers(verifications: &[(PeerId, u64, ValidatorVerification)]) -> Vec<PeerId> {
        verifications
            .iter()
            .map(|(peer_id, _, _)| *peer_id)
            .collect()
    }

    /// The peer IDs of `contacts`.
    fn peer_ids<'a>(contacts: impl IntoIterator<Item = &'a Arc<PeerContactInfo>>) -> Vec<PeerId> {
        contacts
            .into_iter()
            .map(|contact| *contact.peer_id())
            .collect()
    }

    #[test]
    fn the_recheck_sweep_passes_over_a_validators_claims_once_its_share_verified() {
        let pool = pending_pool(&[[1; 10].as_slice(), &[2; 3]].concat());
        let verifier = VerifiesOnly::new(peer_ids(&pool));
        let budget = ValidatorClaimBudget::with_refresh_reserve(5, 5).with_per_validator_limit(4);

        let mut cursor = None;
        let recheck = recheck_pending(&mut cursor, &pool, 128, &verifier, &budget);

        // Four claims of validator 1 and all three of validator 2 were checked. The other claims
        // of validator 1 were looked at, but passed over without a check.
        let expected = peer_ids(pool[..4].iter().chain(&pool[10..]));
        assert_eq!(checked_peers(&recheck.verifications), expected);
        assert_eq!(verifier.calls(), 7);
        assert_eq!(recheck.looked_at, pool.len());
        assert!(pool
            .iter()
            .all(|contact| recheck.has_looked_at(contact.peer_id())));
        assert_eq!(cursor, Some(*pool[12].peer_id()));

        // A sweep starts after the cursor, here a claim of validator 1, and gives that validator's
        // share to the claims it reaches first.
        let recheck = recheck_pending(
            &mut Some(*pool[3].peer_id()),
            &pool,
            128,
            &verifier,
            &budget,
        );
        let expected_after_cursor = peer_ids(pool[4..8].iter().chain(&pool[10..]));
        assert_eq!(checked_peers(&recheck.verifications), expected_after_cursor);

        // The claims passed over do not count towards the checks the sweep may make.
        let recheck = recheck_pending(&mut None, &pool, 7, &verifier, &budget);
        assert_eq!(checked_peers(&recheck.verifications), expected);
    }

    // Claims naming a validator can be made up without its key, and placed anywhere in the pool,
    // which is ordered by peer ID, by generating keys. Placed just before the validator's own
    // claim, behind enough claims of others to use up a tick's checks, they must not keep that
    // claim from being checked on the next tick.
    #[test]
    fn made_up_claims_placed_just_before_a_validators_own_claim_do_not_keep_it_from_being_checked()
    {
        let seeds: Vec<u8> = (2..130).chain([1; 9]).collect();
        let mut pool = pending_pool(&seeds);
        let own = *pool[136].peer_id();
        let own_timestamp = pool[136].contact().timestamp();
        let verifier = VerifiesOnly::new([own]);
        let budget = ValidatorClaimBudget::with_refresh_reserve(5, 5)
            .with_per_validator_limit(Behaviour::VERIFIED_CLAIMS_PER_VALIDATOR_PER_TICK);

        let mut cursor = None;
        let first = recheck_pending(
            &mut cursor,
            &pool,
            Behaviour::MAX_RECHECKED_CLAIMS_PER_TICK,
            &verifier,
            &budget,
        );
        assert_eq!(
            first.verifications.len(),
            Behaviour::MAX_RECHECKED_CLAIMS_PER_TICK
        );
        // The claims checked turned out to be bogus, so they are not pending anymore.
        let checked = checked_peers(&first.verifications);
        pool.retain(|contact| !checked.contains(contact.peer_id()));

        let second = recheck_pending(
            &mut cursor,
            &pool,
            Behaviour::MAX_RECHECKED_CLAIMS_PER_TICK,
            &verifier,
            &budget,
        );
        assert_eq!(
            second.verifications[8],
            (own, own_timestamp, ValidatorVerification::Verified)
        );
    }

    // Checked claims leave the pool before the next tick. The next sweep must continue after the
    // last contact looked at, not at the same position in the changed pool, which would skip the
    // contacts that moved into the positions already looked at.
    #[test]
    fn the_recheck_sweep_continues_after_the_last_contact_it_looked_at() {
        let seeds: Vec<u8> = (0..300).map(|seed| seed as u8).collect();
        let mut pool = pending_pool(&seeds);
        let all = peer_ids(&pool);
        let verifier = VerifiesOnly::default();
        let budget = ValidatorClaimBudget::with_refresh_reserve(5, 5).with_per_validator_limit(4);

        let mut cursor = None;
        let mut checked = Vec::new();
        for tick in 0..3 {
            let recheck = recheck_pending(&mut cursor, &pool, 128, &verifier, &budget);
            let peers = checked_peers(&recheck.verifications);
            let expected_len = if tick < 2 { 128 } else { 44 };
            assert_eq!(peers, all[128 * tick..128 * tick + expected_len]);
            pool.retain(|contact| !peers.contains(contact.peer_id()));
            checked.extend(peers);
        }
        assert_eq!(checked, all);
        assert!(pool.is_empty());
    }

    #[test]
    fn a_recheck_sweep_that_stopped_for_a_node_wide_reason_continues_with_the_claim_it_stopped_at()
    {
        let pool = pending_pool(&[1, 2, 3, 4]);
        let cannot_verify =
            ValidatorVerification::Unverifiable(UnverifiableReason::StateIncomplete);
        let verifier = ScriptedVerifier::new(vec![
            ValidatorVerification::Verified,
            cannot_verify,
            cannot_verify,
            ValidatorVerification::Verified,
            ValidatorVerification::Verified,
        ]);
        let budget = ValidatorClaimBudget::with_refresh_reserve(5, 5).with_per_validator_limit(4);

        let mut cursor = None;
        let first = recheck_pending(&mut cursor, &pool, 128, &verifier, &budget);
        assert!(first.stopped());
        assert_eq!(checked_peers(&first.verifications), peer_ids(&pool[..2]));
        assert_eq!(first.looked_at, 2);
        assert_eq!(cursor, Some(*pool[0].peer_id()));

        // Stopping at the first claim it looks at leaves the cursor where it was.
        let second = recheck_pending(&mut cursor, &pool, 128, &verifier, &budget);
        assert!(second.stopped());
        assert_eq!(checked_peers(&second.verifications), peer_ids(&pool[1..2]));
        assert_eq!(cursor, Some(*pool[0].peer_id()));

        let third = recheck_pending(&mut cursor, &pool, 2, &verifier, &budget);
        assert_eq!(checked_peers(&third.verifications), peer_ids(&pool[1..3]));
    }

    // The sweep keeps its own share, but claims that arrivals already ruled out for this tick
    // must still be passed over: those of a peer that verified a newer contact, and those of
    // any further peer of a validator whose share arrivals used up. A peer that verified an older
    // contact on arrival does not rule out its stored, newer one.
    #[test]
    fn the_recheck_sweep_passes_over_claims_the_share_of_arriving_contacts_rules_out() {
        let pool = pending_pool(&[1, 1]);
        let address = Address::from([1u8; 20]);
        let verifier = VerifiesOnly::new(peer_ids(&pool));
        let (first, second) = (&pool[0], &pool[1]);

        let budget = ValidatorClaimBudget::with_refresh_reserve(5, 5).with_per_validator_limit(4);
        budget.record_verified(
            &address,
            *first.peer_id(),
            first.contact().timestamp() - 60,
            false,
        );
        let (verifications, _) = Behaviour::recheck_validator_claims(
            &pool,
            usize::MAX,
            &verifier,
            &budget,
            &mut HashMap::new(),
        );
        assert_eq!(checked_peers(&verifications), peer_ids(&pool));

        let budget = ValidatorClaimBudget::with_refresh_reserve(5, 5).with_per_validator_limit(4);
        budget.record_verified(
            &address,
            *first.peer_id(),
            first.contact().timestamp() + 1,
            false,
        );
        let (verifications, _) = Behaviour::recheck_validator_claims(
            &pool,
            usize::MAX,
            &verifier,
            &budget,
            &mut HashMap::new(),
        );
        assert_eq!(checked_peers(&verifications), [*second.peer_id()]);

        let budget = ValidatorClaimBudget::with_refresh_reserve(5, 5).with_per_validator_limit(1);
        budget.record_verified(
            &address,
            PeerId::random(),
            first.contact().timestamp(),
            false,
        );
        let (verifications, looked_at) = Behaviour::recheck_validator_claims(
            &pool,
            usize::MAX,
            &verifier,
            &budget,
            &mut HashMap::new(),
        );
        assert!(verifications.is_empty());
        assert_eq!(looked_at, 2);
        assert_eq!(verifier.calls(), 3);
    }

    // The sweep does not remember a set of the contacts it looked at, which could be the whole
    // pool, but the range it went through.
    #[test]
    fn a_recheck_sweep_knows_which_contacts_it_looked_at() {
        let pool = pending_pool(&[1, 2, 3, 4, 5]);
        let verifier = VerifiesOnly::default();
        let budget = ValidatorClaimBudget::with_refresh_reserve(5, 5).with_per_validator_limit(4);

        let mut cursor = Some(*pool[2].peer_id());
        let recheck = recheck_pending(&mut cursor, &pool, 3, &verifier, &budget);
        assert_eq!(
            checked_peers(&recheck.verifications),
            peer_ids([&pool[3], &pool[4], &pool[0]])
        );
        let looked_at: Vec<_> = pool
            .iter()
            .map(|contact| recheck.has_looked_at(contact.peer_id()))
            .collect();
        assert_eq!(looked_at, [true, false, false, true, true]);
        assert!(!recheck.has_looked_at(&PeerId::random()));

        let empty = recheck_pending(&mut None, &[], 3, &verifier, &budget);
        assert!(!empty.has_looked_at(pool[0].peer_id()));
    }

    // A binding kept under a pending refresh is both stale and pending. Its claim must be checked
    // only once per tick.
    #[test(tokio::test(start_paused = true))]
    async fn a_pending_refresh_of_a_stale_binding_is_checked_once_per_tick() {
        let keypair = Keypair::generate_ed25519();
        let book = Arc::new(RwLock::new(PeerContactBook::new(
            signed_contact(&keypair, now_secs(), false),
            false,
            true,
            true,
        )));
        let peer_key = Keypair::generate_ed25519();
        book.write().insert(CheckedPeerContact::check(
            signed_contact(
                &peer_key,
                now_secs() - PeerContactBook::STALE_BINDING_AGE - 10,
                true,
            ),
            &ScriptedVerifier::new(vec![ValidatorVerification::Verified]),
        ));
        book.write()
            .insert(CheckedPeerContact::pending(signed_contact(
                &peer_key,
                now_secs(),
                true,
            )));
        assert_eq!(book.read().unverified_validator_contacts().len(), 1);
        assert_eq!(book.read().stale_verified_validator_contacts().len(), 1);

        let verifier = Arc::new(ScriptedVerifier::new(vec![ValidatorVerification::Verified]));
        let mut behaviour = Behaviour::new(
            Config::new(Blake2bHash::default(), Services::all(), false),
            keypair,
            Arc::clone(&book),
            verifier.clone(),
        );
        run_house_keeping_tick(&mut behaviour).await;
        assert_eq!(verifier.calls(), 1);
        assert!(book
            .read()
            .get(&peer_key.public().to_peer_id())
            .unwrap()
            .is_gossipable());
    }

    // A pending refresh that is itself older than `STALE_BINDING_AGE` keeps the binding stale when
    // it verifies. When the pending claims are re-checked first, that check must still count as
    // the binding's re-check, or the binding would be checked again on the next tick.
    #[test(tokio::test(start_paused = true))]
    async fn a_stale_binding_whose_pending_refresh_the_sweep_checked_is_not_checked_again_soon() {
        let keypair = Keypair::generate_ed25519();
        let book = Arc::new(RwLock::new(PeerContactBook::new(
            signed_contact(&keypair, now_secs(), false),
            false,
            true,
            true,
        )));
        let peer_key = Keypair::generate_ed25519();
        let stale = now_secs() - PeerContactBook::STALE_BINDING_AGE;
        book.write().insert(CheckedPeerContact::check(
            signed_contact(&peer_key, stale - 20, true),
            &ScriptedVerifier::new(vec![ValidatorVerification::Verified]),
        ));
        book.write()
            .insert(CheckedPeerContact::pending(signed_contact(
                &peer_key,
                stale - 10,
                true,
            )));

        let verifier = Arc::new(ScriptedVerifier::new(vec![ValidatorVerification::Verified]));
        let mut behaviour = Behaviour::new(
            Config::new(Blake2bHash::default(), Services::all(), false),
            keypair,
            Arc::clone(&book),
            verifier.clone(),
        );
        run_house_keeping_tick(&mut behaviour).await;
        assert_eq!(verifier.calls(), 1);
        run_house_keeping_tick(&mut behaviour).await;
        assert_eq!(verifier.calls(), 1);
    }

    // A house-keeping tick remembers where its sweep left off, for the next tick to continue there
    // (see `the_recheck_sweep_continues_after_the_last_contact_it_looked_at`).
    #[test(tokio::test(start_paused = true))]
    async fn a_house_keeping_tick_remembers_where_its_recheck_sweep_left_off() {
        let keypair = Keypair::generate_ed25519();
        let book = Arc::new(RwLock::new(PeerContactBook::new(
            signed_contact(&keypair, now_secs(), false),
            false,
            true,
            true,
        )));
        let claims = Behaviour::MAX_RECHECKED_CLAIMS_PER_TICK + 2;
        for seed in 0..claims {
            pending_claim(&mut book.write(), seed as u8);
        }
        let pool = book.read().unverified_validator_contacts();
        let verifier = Arc::new(VerifiesOnly::default());
        let mut behaviour = Behaviour::new(
            Config::new(Blake2bHash::default(), Services::all(), false),
            keypair,
            Arc::clone(&book),
            verifier.clone(),
        );

        run_house_keeping_tick(&mut behaviour).await;
        assert_eq!(verifier.calls(), Behaviour::MAX_RECHECKED_CLAIMS_PER_TICK);
        assert_eq!(
            behaviour.validator_recheck_cursor,
            Some(*pool[Behaviour::MAX_RECHECKED_CLAIMS_PER_TICK - 1].peer_id())
        );
        assert_eq!(book.read().unverified_validator_contacts().len(), 2);

        run_house_keeping_tick(&mut behaviour).await;
        assert_eq!(verifier.calls(), claims);
        assert!(book.read().unverified_validator_contacts().is_empty());
    }

    // A check that shows that this node cannot verify any claim right now is not a re-check: the
    // stale binding it was for is checked again on the next tick.
    #[test(tokio::test(start_paused = true))]
    async fn a_stale_binding_whose_check_found_nothing_out_is_rechecked_on_the_next_tick() {
        let keypair = Keypair::generate_ed25519();
        let book = Arc::new(RwLock::new(PeerContactBook::new(
            signed_contact(&keypair, now_secs(), false),
            false,
            true,
            true,
        )));
        book.write().insert(CheckedPeerContact::check(
            signed_contact(
                &Keypair::generate_ed25519(),
                now_secs() - PeerContactBook::STALE_BINDING_AGE - 10,
                true,
            ),
            &ScriptedVerifier::new(vec![ValidatorVerification::Verified]),
        ));

        let verifier = Arc::new(ScriptedVerifier::new(vec![
            ValidatorVerification::Unverifiable(UnverifiableReason::StateIncomplete),
            ValidatorVerification::Verified,
        ]));
        let mut behaviour = Behaviour::new(
            Config::new(Blake2bHash::default(), Services::all(), false),
            keypair,
            Arc::clone(&book),
            verifier.clone(),
        );
        run_house_keeping_tick(&mut behaviour).await;
        assert_eq!(verifier.calls(), 1);
        run_house_keeping_tick(&mut behaviour).await;
        assert_eq!(verifier.calls(), 2);
    }

    // Both passes of a tick count against the same share of verified claims per validator. Once
    // the pending pass verified a validator's share, a pending refresh of its stale binding that
    // the pending pass did not get to is passed over by the stale pass as well.
    #[test(tokio::test(start_paused = true))]
    async fn the_stale_pass_uses_the_share_the_pending_pass_used() {
        let keypair = Keypair::generate_ed25519();
        let book = Arc::new(RwLock::new(PeerContactBook::new(
            signed_contact(&keypair, now_secs(), false),
            false,
            true,
            true,
        )));
        let checks = Behaviour::MAX_RECHECKED_CLAIMS_PER_TICK;
        let share = Behaviour::VERIFIED_CLAIMS_PER_VALIDATOR_PER_TICK;
        let mut keypairs: Vec<_> = (0..=checks).map(|_| Keypair::generate_ed25519()).collect();
        keypairs.sort_by_key(|keypair| keypair.public().to_peer_id());
        // The last peer, which the pending pass does not get to, has a stale binding to the
        // validator at [1; 20], kept under a pending refresh.
        let stale_key = keypairs.pop().unwrap();
        let stale = now_secs() - PeerContactBook::STALE_BINDING_AGE - 10;
        book.write().insert(CheckedPeerContact::check(
            signed_contact(&stale_key, stale, true),
            &ScriptedVerifier::new(vec![ValidatorVerification::Verified]),
        ));
        book.write()
            .insert(CheckedPeerContact::pending(signed_contact(
                &stale_key,
                now_secs(),
                true,
            )));
        // The pending pass verifies the validator's share, and uses up its checks on others.
        for (index, keypair) in keypairs.iter().enumerate() {
            let seed = if index < share { 1 } else { 2 + index as u8 };
            pending_claim_of(&mut book.write(), keypair, seed);
        }
        let valid = keypairs[..share]
            .iter()
            .chain([&stale_key])
            .map(|keypair| keypair.public().to_peer_id());
        let verifier = Arc::new(VerifiesOnly::new(valid));
        let mut behaviour = Behaviour::new(
            Config::new(Blake2bHash::default(), Services::all(), false),
            keypair,
            Arc::clone(&book),
            verifier.clone(),
        );
        assert_eq!(
            book.read().unverified_validator_contacts().len(),
            checks + 1
        );
        assert_eq!(book.read().stale_verified_validator_contacts().len(), 1);

        run_house_keeping_tick(&mut behaviour).await;
        assert_eq!(verifier.calls(), checks);
    }

    #[test]
    fn the_stale_binding_window_continues_after_the_last_one() {
        let pool: Vec<_> = (0..5).map(|_| stored_verified_contact()).collect();

        let mut cursor = 0;
        let window = Behaviour::select_window(&mut cursor, &pool, 2);
        assert_eq!(window.len(), 2);
        assert_eq!(cursor, 2);
        let window = Behaviour::select_window(&mut cursor, &pool, 4);
        let peers: Vec<_> = window.iter().map(|contact| *contact.peer_id()).collect();
        let expected: Vec<_> = pool[2..]
            .iter()
            .chain(&pool[..1])
            .map(|c| *c.peer_id())
            .collect();
        assert_eq!(peers, expected);
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

        let results = Behaviour::recheck_validator_claims(
            &window,
            usize::MAX,
            &verifier,
            &budget,
            &mut HashMap::new(),
        )
        .0;

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

        let results = Behaviour::recheck_validator_claims(
            &window,
            usize::MAX,
            &verifier,
            &budget,
            &mut HashMap::new(),
        )
        .0;

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
            assert!(budget.may_verify(&address, &PeerId::random(), timestamp, false));
            budget.record_verified(&address, PeerId::random(), timestamp, false);
        }
        assert!(!budget.may_verify(&address, &PeerId::random(), 0, false));
    }

    // Validators re-sign their contact every tick, so anybody can relay one of the older contacts
    // of a validator's peer to a node that does not know the peer yet, ahead of the current one.
    // The current one must still verify on arrival.
    #[test(tokio::test)]
    async fn the_behaviours_budget_lets_a_fresh_contact_follow_an_older_one_on_arrival() {
        let verifier = ScriptedVerifier::new(vec![ValidatorVerification::Verified; 2]);
        let (behaviour, book, _pending) =
            behaviour_with_pending_claim(Arc::new(ScriptedVerifier::new(vec![])));
        let budget = &behaviour.validator_claim_budget;
        let now = now_secs();
        assert!(budget.is_fresh(now - 59, now));
        assert!(!budget.is_fresh(now - 60, now));

        let peer_key = Keypair::generate_ed25519();
        let peer_id = peer_key.public().to_peer_id();
        for timestamp in [now - 1500, now - 5] {
            let checked = CheckedPeerContact::check_new_with_budget(
                signed_contact(&peer_key, timestamp, true),
                &book,
                &verifier,
                budget,
            );
            book.write().insert(checked);
        }
        assert_eq!(verifier.calls(), 2);
        let stored = book.read().get(&peer_id).unwrap();
        assert_eq!(stored.contact().timestamp(), now - 5);
        assert!(stored.is_gossipable());
    }

    /// The limits that keep a single validator from spending the claim budget of every node are
    /// meant to be small: an honest validator binds one peer, and refreshes its contact once per
    /// tick.
    #[test]
    fn the_limits_on_a_single_validator_are_small() {
        assert_eq!(PeerContactBook::MAX_BINDINGS_PER_VALIDATOR, 4);
        assert_eq!(Behaviour::VERIFIED_CLAIMS_PER_VALIDATOR_PER_TICK, 4);
        // Apart from a short overlap, no more than one regularly re-signed contact of a peer is
        // fresh at a time.
        assert_eq!(
            Behaviour::FRESH_VALIDATOR_CONTACT_AGE,
            std::time::Duration::from_secs(60)
        );
        assert!(
            Behaviour::FRESH_VALIDATOR_CONTACT_AGE
                <= Config::new(Blake2bHash::default(), Services::all(), false)
                    .house_keeping_interval
        );
    }
}
