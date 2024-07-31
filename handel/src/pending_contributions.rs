use core::{
    pin::Pin,
    task::{Context, Poll},
};
use std::{collections::HashSet, fmt, sync::Arc};

use futures::{stream::BoxStream, Stream, StreamExt};

use crate::{
    contribution::AggregatableContribution, evaluator::Evaluator, protocol::Protocol,
    update::LevelUpdate, Identifier,
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

/// The PendingContributionlist. Implements Stream to poll for the next best scoring PendingContribution.
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
    /// The Stream where LevelUpdates can be polled from, which are subsequently converted into one or two
    /// PendingContributions.
    input_stream: BoxStream<'static, LevelUpdate<TProtocol::Contribution>>,
    /// Keeps track on whether or not the PendingContributionList has been polled at least once.
    /// Used to make sure that operations which need to happen before the first poll
    /// are in fact executed before the first poll.
    is_unpolled: bool,
}

impl<TId, TProtocol> PendingContributionList<TId, TProtocol>
where
    TId: Identifier,
    TProtocol: Protocol<TId>,
{
    /// Create a new PendingContributionList:
    /// * `evaluator` - The evaluator which will be used for contribution scoring
    /// * `input_stream` - The stream on which new LevelUpdates can be polled, which will then be converted into PendingContributions
    pub fn new(
        id: TId,
        evaluator: Arc<TProtocol::Evaluator>,
        input_stream: BoxStream<'static, LevelUpdate<TProtocol::Contribution>>,
    ) -> Self {
        Self {
            id,
            list: HashSet::new(),
            evaluator,
            input_stream,
            is_unpolled: true,
        }
    }

    /// Safe without a wake as it is only called in the constructor of Aggregation.
    pub fn add_contribution(&mut self, contribution: TProtocol::Contribution, level: usize) {
        // As no waker is used, this must only ever happen before the first poll.
        assert!(self.is_unpolled);
        // Add the item to the list.
        self.list.insert(PendingContribution {
            contribution,
            level,
        });
    }

    pub fn into_stream(self) -> BoxStream<'static, LevelUpdate<TProtocol::Contribution>> {
        self.input_stream
    }
}

impl<TId, TProtocol> Stream for PendingContributionList<TId, TProtocol>
where
    TId: Identifier,
    TProtocol: Protocol<TId>,
{
    type Item = PendingContribution<TProtocol::Contribution>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Once poll is called this list is no longer unpolled.
        self.is_unpolled = true;

        // current best score
        let mut best_score: usize = 0;
        // retained set of PendingContributions.
        // Same as self.list, but did not retain 0 score PendingContributions and the best PendingContribution.
        let mut new_set: HashSet<Self::Item> = HashSet::new();
        // the current best PendingContribution
        let mut best_pending_contribution: Option<Self::Item> = None;
        // A local copy is needed so mut self is not borrowed
        let ev = Arc::clone(&self.evaluator);

        let id = self.id.clone();

        // Scoring of items needs to be done every time a item is polled, as the scores might have
        // changed with the last aggregated contribution

        // Score already available PendingContributions first
        for item in self.list.drain() {
            // Have the evaluator score each PendingContribution.
            let score = item.evaluate::<TId, TProtocol>(Arc::clone(&ev), id.clone());
            // if an item has a score greater than 0 it is retained. Otherwise it is discarded
            if score > 0 {
                if score > best_score {
                    // In case it's a new best remember it and push the old best into the retained set.
                    if let Some(pending_contribution) = best_pending_contribution {
                        new_set.insert(pending_contribution);
                    }
                    best_pending_contribution = Some(item);
                    // remember the new best score
                    best_score = score;
                } else {
                    // in case the item is not the new best scoring item, push it into the retained set.
                    new_set.insert(item);
                }
            }
        }

        // The list must be empty now since it is going to be overwritten.
        assert!(self.list.is_empty());

        // Update PendingContributions with the the retained list of PendingContributions
        self.list = new_set;

        // Scan the input for better PendingContributions. The loop exits once the input has run out of LevelUpdates.
        // Note that computations are limited to the bare minimum. No verification in particular.
        // As Verification is very computationally expensive it should only be done for the PendingContribution with the highest score.
        while let Poll::Ready(Some(msg)) = self.input_stream.poll_next_unpin(cx) {
            // TODO the case where the msg is None is not being handled which could mean that:
            // The input has ended, i.e. there is no producer left.
            // In Test cases that could mean the other instances have completed their aggregations and dropped their network instances.
            // In reality this should never happen as the network should not terminate those streams, but try to acquire new Peers in this situation.
            // Panic here is viable, but makes testing a bit harder.
            // TODO more robust handling of this case, as the aggregation might not be able to finish here (depending on what PendingContributions are left).

            // A new LevelUpdate is available when the msg is Some:
            if self.evaluator.level_contains_origin(&msg) {
                // Every LevelUpdate contains an aggregate which can be turned into a PendingContribution.
                let pending_aggregate_contribution = PendingContribution {
                    contribution: msg.aggregate,
                    level: msg.level as usize,
                };
                // Score the newly created PendingContribution for the aggregate of the LevelUpdate
                let score = pending_aggregate_contribution
                    .evaluate::<TId, TProtocol>(Arc::clone(&self.evaluator), id.clone());
                // PendingContributions with a score of 0 are discarded (meaning not added to the set).
                if score > 0 {
                    trace!(
                        id = ?self.id,
                        ?score,
                        ?pending_aggregate_contribution,
                        "New PendingContribution",
                    );
                    if score > best_score {
                        // If the score is a new best remember the score and put the former best item into the list.
                        best_score = score;
                        if let Some(pending_contribution) = best_pending_contribution {
                            self.list.insert(pending_contribution);
                        }
                        best_pending_contribution = Some(pending_aggregate_contribution);
                    } else {
                        // If the score is not a new best put the PendingContribution in the list.
                        self.list.insert(pending_aggregate_contribution);
                    }
                }
                // Some of the LevelUpdates also contain an individual Signature in which case it is also converted into a PendingContribution.
                if let Some(individual) = msg.individual {
                    let pending_individual_contribution = PendingContribution {
                        contribution: individual,
                        level: msg.level as usize,
                    };
                    // Score the newly created PendingContribution for the individual contribution of the LevelUpdate.
                    let score = pending_individual_contribution
                        .evaluate::<TId, TProtocol>(Arc::clone(&self.evaluator), self.id.clone());
                    // PendingContributions with a score of 0 are discarded (meaning not added to the set).
                    if score > 0 {
                        if score > best_score {
                            // If the score is a new best remember the score and put the former best item into the list.
                            best_score = score;
                            if let Some(pending_contribution) = best_pending_contribution {
                                self.list.insert(pending_contribution);
                            }
                            best_pending_contribution = Some(pending_individual_contribution);
                        } else {
                            // If the score is not a new best put the PendingContribution in the list.
                            self.list.insert(pending_individual_contribution);
                        }
                    }
                }
            } else {
                trace!(
                    sender = ?msg.origin,
                    level = ?msg.level,
                    "Sender of update is not on the correct level",
                );
            }
        }

        // If the best item has a score higher than 0 return it otherwise signal
        // that no new aggregate is currently available.
        if best_score > 0 {
            // The function returns Poll<Option<PendingContribution<C>>> but Ready(None) is never returned.
            return Poll::Ready(Some(best_pending_contribution.expect(
                "Score was higher than 0 but there was no best PendingContribution.",
            )));
        }

        Poll::Pending
    }
}
