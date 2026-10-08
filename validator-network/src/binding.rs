//! Peer ID bindings of validators and the rule that orders them.

/// A validator's peer ID and how we learned it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding<TPeerId> {
    pub peer_id: TPeerId,
    /// Timestamp (ms) of the signed record, `None` for the unsigned fallback list.
    pub signed_at: Option<u64>,
}

impl<TPeerId: Copy + PartialEq> Binding<TPeerId> {
    pub fn signed(peer_id: TPeerId, timestamp: u64) -> Self {
        Self {
            peer_id,
            signed_at: Some(timestamp),
        }
    }

    pub fn unsigned(peer_id: TPeerId) -> Self {
        Self {
            peer_id,
            signed_at: None,
        }
    }

    /// Signed beats unsigned, a later timestamp beats an earlier one.
    pub fn is_newer_than(&self, other: &Self) -> bool {
        match (self.signed_at, other.signed_at) {
            (Some(ours), Some(theirs)) => ours > theirs,
            (Some(_), None) => true,
            (None, _) => false,
        }
    }
}

/// Cache slot of one validator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheState<TPeerId> {
    /// Bound to a peer.
    Resolved(Binding<TPeerId>),
    /// Resolution failed; the previous binding may still work.
    Error(Option<Binding<TPeerId>>),
    /// Resolution running; the previous binding may still work.
    InProgress(Option<Binding<TPeerId>>),
    /// A request to the bound peer failed; the binding may be outdated.
    Empty(Binding<TPeerId>),
}

impl<TPeerId: Copy + PartialEq> CacheState<TPeerId> {
    /// The binding if the slot is resolved.
    pub fn current(&self) -> Option<Binding<TPeerId>> {
        match *self {
            CacheState::Resolved(binding) => Some(binding),
            _ => None,
        }
    }

    /// The newest binding known, possibly outdated.
    pub fn latest(&self) -> Option<Binding<TPeerId>> {
        match *self {
            CacheState::Resolved(binding) | CacheState::Empty(binding) => Some(binding),
            CacheState::Error(previous) | CacheState::InProgress(previous) => previous,
        }
    }

    /// Takes `binding` if newer than a resolved one, or at least as new as a failed one.
    /// Returns whether the slot is now bound.
    pub fn offer(&mut self, binding: Binding<TPeerId>) -> bool {
        let taken = match *self {
            CacheState::Resolved(current) => binding.is_newer_than(&current),
            CacheState::Empty(previous) => !previous.is_newer_than(&binding),
            CacheState::Error(previous) | CacheState::InProgress(previous) => {
                previous.is_none_or(|previous| !previous.is_newer_than(&binding))
            }
        };
        if taken {
            *self = CacheState::Resolved(binding);
        }
        taken
    }

    /// Starts a resolution unless one is running or the slot is resolved; returns whether it did.
    pub fn start_resolving(&mut self) -> bool {
        let previous = match *self {
            CacheState::Error(previous) => previous,
            CacheState::Empty(previous) => Some(previous),
            CacheState::Resolved(_) | CacheState::InProgress(_) => return false,
        };
        *self = CacheState::InProgress(previous);
        true
    }

    /// Applies a resolution outcome like an offer; a rejected outcome fails a running resolution.
    pub fn resolved(&mut self, outcome: Option<Binding<TPeerId>>) {
        if let CacheState::InProgress(previous) = *self {
            *self = match outcome {
                Some(binding)
                    if previous.is_none_or(|previous| !previous.is_newer_than(&binding)) =>
                {
                    CacheState::Resolved(binding)
                }
                _ => CacheState::Error(previous),
            };
        } else if let Some(binding) = outcome {
            self.offer(binding);
        }
    }

    /// Drops the binding to `peer_id` after a failed request; returns whether it did.
    pub fn mark_failed(&mut self, peer_id: TPeerId) -> bool {
        match *self {
            CacheState::Resolved(binding) if binding.peer_id == peer_id => {
                *self = CacheState::Empty(binding);
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Binding, CacheState};

    const P1: u8 = 1;
    const P2: u8 = 2;
    const P3: u8 = 3;

    fn signed(peer_id: u8, timestamp: u64) -> Binding<u8> {
        Binding::signed(peer_id, timestamp)
    }

    fn unsigned(peer_id: u8) -> Binding<u8> {
        Binding::unsigned(peer_id)
    }

    #[test]
    fn a_signed_binding_is_newer_than_an_unsigned_one() {
        assert!(signed(P1, 1).is_newer_than(&unsigned(P2)));
        assert!(signed(P1, 0).is_newer_than(&unsigned(P1)));
    }

    #[test]
    fn a_later_timestamp_is_newer() {
        assert!(signed(P2, 2).is_newer_than(&signed(P1, 1)));
        assert!(signed(P1, 2).is_newer_than(&signed(P1, 1)));
    }

    #[test]
    fn an_earlier_timestamp_is_not_newer() {
        assert!(!signed(P1, 1).is_newer_than(&signed(P2, 2)));
    }

    #[test]
    fn an_equal_timestamp_is_not_newer() {
        assert!(!signed(P2, 1).is_newer_than(&signed(P1, 1)));
        assert!(!signed(P1, 1).is_newer_than(&signed(P1, 1)));
    }

    #[test]
    fn an_unsigned_binding_is_never_newer() {
        assert!(!unsigned(P2).is_newer_than(&unsigned(P1)));
        assert!(!unsigned(P2).is_newer_than(&signed(P1, 0)));
    }

    #[test]
    fn a_newer_offer_replaces_a_resolved_binding() {
        let mut slot = CacheState::Resolved(signed(P1, 1));
        assert!(slot.offer(signed(P2, 2)));
        assert_eq!(slot, CacheState::Resolved(signed(P2, 2)));
    }

    #[test]
    fn an_older_or_equal_offer_keeps_a_resolved_binding() {
        for offer in [signed(P2, 1), signed(P2, 0), unsigned(P2)] {
            let mut slot = CacheState::Resolved(signed(P1, 1));
            assert!(!slot.offer(offer), "{offer:?}");
            assert_eq!(slot, CacheState::Resolved(signed(P1, 1)));
        }
    }

    #[test]
    fn an_unsigned_offer_does_not_replace_an_unsigned_binding() {
        let mut slot = CacheState::Resolved(unsigned(P1));
        assert!(!slot.offer(unsigned(P2)));
        assert_eq!(slot, CacheState::Resolved(unsigned(P1)));
    }

    #[test]
    fn an_offer_binds_when_nothing_was_known() {
        for state in [CacheState::InProgress(None), CacheState::Error(None)] {
            for offer in [unsigned(P1), signed(P1, 1)] {
                let mut slot = state;
                assert!(slot.offer(offer), "{state:?} {offer:?}");
                assert_eq!(slot, CacheState::Resolved(offer));
            }
        }
    }

    #[test]
    fn an_offer_at_least_as_new_as_a_failed_binding_rebinds() {
        let failed = signed(P1, 1);
        for state in [
            CacheState::Empty(failed),
            CacheState::Error(Some(failed)),
            CacheState::InProgress(Some(failed)),
        ] {
            for offer in [signed(P1, 1), signed(P2, 1), signed(P2, 2)] {
                let mut slot = state;
                assert!(slot.offer(offer), "{state:?} {offer:?}");
                assert_eq!(slot, CacheState::Resolved(offer));
            }
        }
    }

    #[test]
    fn an_offer_older_than_a_failed_binding_is_ignored() {
        let failed = signed(P1, 1);
        for state in [
            CacheState::Empty(failed),
            CacheState::Error(Some(failed)),
            CacheState::InProgress(Some(failed)),
        ] {
            for offer in [signed(P2, 0), unsigned(P2)] {
                let mut slot = state;
                assert!(!slot.offer(offer), "{state:?} {offer:?}");
                assert_eq!(slot, state);
            }
        }
    }

    #[test]
    fn an_unsigned_offer_rebinds_after_an_unsigned_failure() {
        for state in [
            CacheState::Empty(unsigned(P1)),
            CacheState::Error(Some(unsigned(P1))),
            CacheState::InProgress(Some(unsigned(P1))),
        ] {
            let mut slot = state;
            assert!(slot.offer(unsigned(P2)), "{state:?}");
            assert_eq!(slot, CacheState::Resolved(unsigned(P2)));
        }
    }

    #[test]
    fn a_resolution_starts_only_from_a_failed_or_dropped_slot() {
        let binding = signed(P1, 1);
        for (state, expected) in [
            (
                CacheState::Error(Some(binding)),
                CacheState::InProgress(Some(binding)),
            ),
            (CacheState::Error(None), CacheState::InProgress(None)),
            (
                CacheState::Empty(binding),
                CacheState::InProgress(Some(binding)),
            ),
        ] {
            let mut slot = state;
            assert!(slot.start_resolving(), "{state:?}");
            assert_eq!(slot, expected);
        }
        for state in [
            CacheState::Resolved(binding),
            CacheState::InProgress(Some(binding)),
            CacheState::InProgress(None),
        ] {
            let mut slot = state;
            assert!(!slot.start_resolving(), "{state:?}");
            assert_eq!(slot, state);
        }
    }

    #[test]
    fn a_resolution_result_at_least_as_new_as_the_previous_binding_binds() {
        for (state, outcome) in [
            (CacheState::InProgress(Some(signed(P1, 1))), signed(P1, 1)),
            (CacheState::InProgress(Some(signed(P1, 1))), signed(P2, 2)),
            (CacheState::InProgress(Some(unsigned(P1))), unsigned(P2)),
            (CacheState::InProgress(None), unsigned(P3)),
        ] {
            let mut slot = state;
            slot.resolved(Some(outcome));
            assert_eq!(slot, CacheState::Resolved(outcome), "{state:?}");
        }
    }

    #[test]
    fn a_resolution_result_older_than_the_previous_binding_fails_the_resolution() {
        for outcome in [signed(P2, 1), unsigned(P2)] {
            let mut slot = CacheState::InProgress(Some(signed(P1, 2)));
            slot.resolved(Some(outcome));
            assert_eq!(slot, CacheState::Error(Some(signed(P1, 2))), "{outcome:?}");
        }
    }

    #[test]
    fn a_resolution_without_a_result_fails_the_resolution() {
        for previous in [Some(signed(P1, 1)), None] {
            let mut slot = CacheState::InProgress(previous);
            slot.resolved(None);
            assert_eq!(slot, CacheState::Error(previous));
        }
    }

    #[test]
    fn a_resolution_result_does_not_clobber_a_newer_binding_offered_meanwhile() {
        let mut slot = CacheState::InProgress(Some(signed(P1, 1)));
        assert!(slot.offer(signed(P2, 3)));

        slot.resolved(Some(signed(P1, 2)));
        assert_eq!(slot, CacheState::Resolved(signed(P2, 3)));

        slot.resolved(None);
        assert_eq!(slot, CacheState::Resolved(signed(P2, 3)));
    }

    #[test]
    fn a_resolution_result_newer_than_a_binding_offered_meanwhile_replaces_it() {
        let mut slot = CacheState::InProgress(None);
        assert!(slot.offer(signed(P2, 1)));

        slot.resolved(Some(signed(P3, 2)));
        assert_eq!(slot, CacheState::Resolved(signed(P3, 2)));
    }

    #[test]
    fn outside_a_running_resolution_a_result_is_just_an_offer() {
        // The binding offered meanwhile failed before the resolution came back.
        let mut slot = CacheState::Empty(signed(P1, 2));
        slot.resolved(Some(signed(P2, 1)));
        assert_eq!(slot, CacheState::Empty(signed(P1, 2)));
        slot.resolved(Some(signed(P2, 2)));
        assert_eq!(slot, CacheState::Resolved(signed(P2, 2)));
    }

    #[test]
    fn a_failed_request_drops_only_the_binding_to_the_failed_peer() {
        let binding = signed(P1, 1);
        let mut slot = CacheState::Resolved(binding);
        assert!(!slot.mark_failed(P2));
        assert_eq!(slot, CacheState::Resolved(binding));
        assert!(slot.mark_failed(P1));
        assert_eq!(slot, CacheState::Empty(binding));

        for state in [
            CacheState::Empty(binding),
            CacheState::Error(Some(binding)),
            CacheState::InProgress(Some(binding)),
        ] {
            let mut slot = state;
            assert!(!slot.mark_failed(P1), "{state:?}");
            assert_eq!(slot, state);
        }
    }

    #[test]
    fn current_and_latest_report_the_binding_per_state() {
        let binding = signed(P1, 1);
        for (state, current, latest) in [
            (CacheState::Resolved(binding), Some(binding), Some(binding)),
            (CacheState::Error(Some(binding)), None, Some(binding)),
            (CacheState::Error(None), None, None),
            (CacheState::InProgress(Some(binding)), None, Some(binding)),
            (CacheState::InProgress(None), None, None),
            (CacheState::Empty(binding), None, Some(binding)),
        ] {
            assert_eq!(state.current(), current, "{state:?}");
            assert_eq!(state.latest(), latest, "{state:?}");
        }
    }
}
