use core::pin::Pin;
use core::task::{Context, Poll};
use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;
use std::task::Waker;

use futures::{stream::BoxStream, Stream, StreamExt};

use nimiq_macros::store_waker;

use crate::contribution::AggregatableContribution;
use crate::evaluator::Evaluator;
use crate::update::LevelUpdate;

/// A TodoItem represents a contribution which has not yet been aggregated into the store.
#[derive(Clone)]
pub(crate) struct TodoItem<C: AggregatableContribution> {
    /// The contribution of this TodoItem
    pub contribution: C,
    /// The level the contribution of this TodoItem belongs to.
    pub level: usize,
}

impl<C: AggregatableContribution> fmt::Debug for TodoItem<C> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut dbg = f.debug_struct("TodoItem");
        dbg.field("level", &self.level);
        dbg.field("signers", &self.contribution.contributors());
        dbg.finish()
    }
}

impl<C: AggregatableContribution> TodoItem<C> {
    /// Evaluated the contribution of the TodoItem. It returns a score representing how useful
    /// the contribution is, with `0` meaning not useful at all -> can be discarded and `> 0`
    /// meaning more useful the higher the number.
    ///
    /// * `evaluator` - The evaluator used for the score computation
    pub fn evaluate<E: Evaluator<C>>(&self, evaluator: Arc<E>) -> usize {
        evaluator.evaluate(&self.contribution, self.level)
    }
}

impl<C: AggregatableContribution> PartialEq for TodoItem<C> {
    fn eq(&self, other: &TodoItem<C>) -> bool {
        self.level == other.level
            && self.contribution.contributors() == other.contribution.contributors()
    }
}

impl<C: AggregatableContribution> Eq for TodoItem<C> {}

impl<C: AggregatableContribution> std::hash::Hash for TodoItem<C> {
    // TODO
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(&self.contribution.contributors().to_string(), state);
    }
}

/// TodoItem list. Implements Stream to poll for the next best scoring TodoItem.
/// Will dry the input stream every time a TodoItem is polled.
pub(crate) struct TodoList<C: AggregatableContribution, E: Evaluator<C>> {
    /// List of TodoItems already polled from input stream
    list: HashSet<TodoItem<C>>,
    /// The evaluator used for scoring the individual todos
    evaluator: Arc<E>,
    /// The Stream where LevelUpdates can be polled from, which are subsequently converted into TodoItems
    input_stream: BoxStream<'static, LevelUpdate<C>>,

    waker: Option<Waker>,
}

impl<C: AggregatableContribution, E: Evaluator<C>> TodoList<C, E> {
    /// Create a new TodoList
    /// * `evaluator` - The evaluator which will be used for TodoItem scoring
    /// * `input_stream` - The stream on which new LevelUpdates can be polled, which will then be converted into TodoItems
    pub fn new(evaluator: Arc<E>, input_stream: BoxStream<'static, LevelUpdate<C>>) -> Self {
        Self {
            list: HashSet::new(),
            evaluator,
            input_stream,
            waker: None,
        }
    }

    pub fn add_contribution(&mut self, contribution: C, level: usize) {
        self.list.insert(TodoItem {
            contribution,
            level,
        });
        self.wake();
    }

    fn wake(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }

    pub fn into_stream(self) -> BoxStream<'static, LevelUpdate<C>> {
        self.input_stream
    }
}

impl<C: AggregatableContribution, E: Evaluator<C>> Stream for TodoList<C, E> {
    type Item = TodoItem<C>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // current best score
        let mut best_score: usize = 0;
        // retained set of todos. Same as self.list, but did not retain 0 score todos and the best todo.
        let mut new_set: HashSet<TodoItem<C>> = HashSet::new();
        // the current best TodoItem
        let mut best_todo: Option<Self::Item> = None;
        // A local copy is needed so mut self is not borrowed
        let ev = Arc::clone(&self.evaluator);

        // Scoring of items needs to be done every time a item is polled, as the scores might have
        // changed with the last aggregated contribution

        // Score already available TodoItems first
        for item in self.list.drain() {
            // Have the evaluator score each TodoItem.
            let score = item.evaluate(Arc::clone(&ev));
            // if an item has a score greater than 0 it is retained. Otherwise it is discarded
            if score > 0 {
                if score > best_score {
                    // In case it's a new best remember it and push the old best into the retained set.
                    if let Some(todo) = best_todo {
                        new_set.insert(todo);
                    }
                    best_todo = Some(item);
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

        // Update todos with the the retained list of Todos
        self.list = new_set;

        // Scan the input for better todos. The loop exits once the input has run out of LevelUpdates.
        // Note that computations are limited to the bare minimum. No verification in particular.
        // As Verification is very computationally expensive it should only be done for the TodoItem with the highest score.
        while let Poll::Ready(Some(msg)) = self.input_stream.poll_next_unpin(cx) {
            // TODO the case where the msg is None is not being handled which could mean that:
            // The input has ended, i.e. there is no producer left.
            // In testcases that could mean the other instances have completed their aggregations and droped their network instances.
            // In reality this should never happen as the network should not terminate those streams, but try to aquire new Peers in this situation.
            // Panic here is viable, but makes testing a bit harder.
            // TODO more robust handling of this case, as the aggregation might not be able to finish here (depending on what todos are left).

            // A new LevelUpdate is available when the msg is Some:
            if self
                .evaluator
                .level_contains_id(msg.level as usize, msg.origin as usize)
            {
                // Every LevelUpdate contains an aggregate which can be turned into a TodoItem
                let aggregate_todo = TodoItem {
                    contribution: msg.aggregate,
                    level: msg.level as usize,
                };
                // Score the newly created TodoItem for the aggregate of the LevelUpdate
                let score = aggregate_todo.evaluate(Arc::clone(&self.evaluator));
                // TodoItems with a score of 0 are discarded (meaning not added to the set of TodoItems).
                if score > 0 {
                    trace!("New todo with score: {}", &score);
                    if score > best_score {
                        // If the score is a new best remember the score and put the former best item into the list.
                        best_score = score;
                        if let Some(best_todo) = best_todo {
                            self.list.insert(best_todo);
                            self.wake();
                        }
                        best_todo = Some(aggregate_todo);
                    } else {
                        // If the score is not a new best put the TodoItem in the list.
                        self.list.insert(aggregate_todo);
                        self.wake();
                    }
                }
                // Some of the LevelUpdates also contain an individual Signature in which case it is also converted into a TodoItem.
                if let Some(individual) = msg.individual {
                    let individual_todo = TodoItem {
                        contribution: individual,
                        level: msg.level as usize,
                    };
                    // Score the newly created TodoItem for the individual contribution of the LevelUpdate.
                    let score = individual_todo.evaluate(Arc::clone(&self.evaluator));
                    // TodoItems with a score of 0 are discarded (meaning not added to the set of TodoItems).
                    if score > 0 {
                        if score > best_score {
                            // If the score is a new best remember the score and put the former best item into the list.
                            best_score = score;
                            if let Some(best_todo) = best_todo {
                                self.list.insert(best_todo);
                                self.wake();
                            }
                            best_todo = Some(individual_todo);
                        } else {
                            // If the score is not a new best put the TodoItem in the list.
                            self.list.insert(individual_todo);
                            self.wake();
                        }
                    }
                }
            } else {
                debug!(
                    "Sender of update :{} is not on level {}",
                    msg.origin, msg.level
                );
            }
        }

        // If the best item has a score higher than 0 return it otherwise signal
        // that no TodoItem is currently available.
        // The function returns Poll<Option<TodoItem<C>>> but Ready(None) is never returned.
        if best_score > 0 {
            // best_todo is now always Some(todo) never None
            if let Some(todo) = best_todo {
                Poll::Ready(Some(todo))
            } else {
                unreachable!(" Score was higher than 0 but there was no best TodoItem.");
            }
        } else {
            store_waker!(self, waker, cx);
            Poll::Pending
        }
    }
}
