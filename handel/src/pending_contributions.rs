use core::{
    pin::Pin,
    task::{Context, Poll},
};
use std::{collections::HashSet, fmt, sync::Arc, task::Waker};

use futures::Stream;
use nimiq_utils::WakerExt;

use crate::{
    contribution::AggregatableContribution, evaluator::Evaluator, protocol::Protocol, Identifier,
};

/// A PendingContribution represents a contribution which has not yet been aggregated into the store.
/// For the most part a wrapper for the contribution within.
#[derive(Clone)]
pub(crate) struct PendingContribution<C: AggregatableContribution> {
    /// The pending contribution
    pub contribution: C,
    /// The level the contribution belongs to.
    pub level: usize,
}

impl<C: AggregatableContribution> fmt::Debug for PendingContribution<C> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut dbg = f.debug_struct("PendingContribution");
        dbg.field("level", &self.level);
        dbg.field("signers", &self.contribution.contributors());
        dbg.finish()
    }
}

impl<C: AggregatableContribution> PendingContribution<C> {
    /// Evaluates the contribution of the PendingContribution. It returns a score representing how useful
    /// the contribution is, with `0` meaning not useful at all -> can be discarded and `> 0`
    /// meaning more useful the higher the number.
    ///
    /// * `evaluator` - The evaluator used for the score computation
    pub fn evaluate<TId: Identifier, TProtocol: Protocol<TId, Contribution = C>>(
        &self,
        evaluator: Arc<TProtocol::Evaluator>,
        id: TId,
    ) -> usize {
        evaluator.evaluate(&self.contribution, self.level, id)
    }
}

impl<C: AggregatableContribution> PartialEq for PendingContribution<C> {
    fn eq(&self, other: &PendingContribution<C>) -> bool {
        self.level == other.level
            && self.contribution.contributors() == other.contribution.contributors()
    }
}

impl<C: AggregatableContribution> Eq for PendingContribution<C> {}

impl<C: AggregatableContribution> std::hash::Hash for PendingContribution<C> {
    // TODO
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(&self.level, state);
        std::hash::Hash::hash(&self.contribution.contributors().to_string(), state);
    }
}

/// Implements Stream to poll for the next best scoring PendingContribution.
/// Will dry the input stream every time a PendingContribution is polled.
pub(crate) struct PendingContributionList<TId, TProtocol>
where
    TId: Identifier,
    TProtocol: Protocol<TId>,
{
    /// The ID of this aggregation.
    id: TId,
    /// List of PendingContributions already polled from input stream.
    list: HashSet<PendingContribution<TProtocol::Contribution>>,
    /// The evaluator used for scoring an individual PendingContribution.
    evaluator: Arc<TProtocol::Evaluator>,
    /// Waker to wake the task when a contribution is added manually.
    waker: Option<Waker>,
}

impl<TId, TProtocol> PendingContributionList<TId, TProtocol>
where
    TId: Identifier,
    TProtocol: Protocol<TId>,
{
    /// Create a new PendingContributionList:
    /// * `evaluator` - The evaluator which will be used for contribution scoring
    /// * `input_stream` - The stream on which new LevelUpdates can be polled, which will then be converted into PendingContributions
    pub fn new(id: TId, evaluator: Arc<TProtocol::Evaluator>) -> Self {
        Self {
            id,
            list: HashSet::new(),
            evaluator,
            waker: None,
        }
    }

    pub fn add_contribution(&mut self, contribution: TProtocol::Contribution, level: usize) {
        // Add the item to the list.
        self.list.insert(PendingContribution {
            contribution,
            level,
        });

        // Wake the task to process this contribution.
        self.waker.wake();
    }
}

impl<TId, TProtocol> Stream for PendingContributionList<TId, TProtocol>
where
    TId: Identifier,
    TProtocol: Protocol<TId>,
{
    type Item = PendingContribution<TProtocol::Contribution>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // The current best score.
        let mut best_score: usize = 0;
        // The current best PendingContribution.
        let mut best_contribution: Option<Self::Item> = None;
        // Retained set of PendingContributions. Same as `self.list`, but does not retain 0-score
        // PendingContributions and the best PendingContribution.
        let mut retain_set: HashSet<Self::Item> = HashSet::new();
        // A local copy is needed so self is not borrowed.
        let evaluator = Arc::clone(&self.evaluator);
        let id = self.id.clone();

        // Scoring of items needs to be done every time an item is polled, as the scores might have
        // changed with the last aggregated contribution.

        // Score already available PendingContributions first.
        for item in self.list.drain() {
            // Use the evaluator to compute the score of the contribution.
            let score = item.evaluate::<TId, TProtocol>(Arc::clone(&evaluator), id.clone());

            // If an item has a score of 0, it is discarded.
            if score == 0 {
                continue;
            }

            // Check if this item is the new best.
            if score > best_score {
                // Push the old best into the retained set.
                if let Some(old_best) = best_contribution.take() {
                    retain_set.insert(old_best);
                }
                // Remember the new best.
                best_score = score;
                best_contribution = Some(item);
            } else {
                // In case the item is not the new best, push it into the retained set.
                retain_set.insert(item);
            }
        }

        // Update PendingContributions with the retained list of PendingContributions.
        self.list = retain_set;

        // Return the best contribution if we have one.
        if best_contribution.is_some() {
            return Poll::Ready(best_contribution);
        }

        // Wait for more contributions.
        self.waker.store_waker(cx);
        Poll::Pending
    }
}
