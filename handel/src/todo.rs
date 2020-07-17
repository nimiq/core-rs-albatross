use std::sync::Arc;

use parking_lot::{RwLock, RwLockUpgradableReadGuard};

use crate::contribution::AggregatableContribution;
use crate::evaluator::Evaluator;

#[derive(Clone, Debug)]
struct TodoItem<C: AggregatableContribution> {
    contribution: C,
    level: usize,
}

impl<C: AggregatableContribution> TodoItem<C> {
    pub fn evaluate<E: Evaluator<C>>(&self, evaluator: Arc<E>) -> usize {
        evaluator.evaluate(&self.contribution, self.level)
    }
}

#[derive(Debug)]
pub(crate) struct TodoList<C: AggregatableContribution, E: Evaluator<C>> {
    list: RwLock<Vec<TodoItem<C>>>,
    evaluator: Arc<E>,
    //task: Option<Task>
}

impl<C: AggregatableContribution, E: Evaluator<C>> TodoList<C, E> {
    pub fn new(evaluator: Arc<E>) -> Self {
        Self {
            list: RwLock::new(Vec::new()),
            evaluator,
            //task: None,
        }
    }

    pub fn put(&self, contribution: C, level: usize) {
        trace!(
            "Putting {:?} (level {}) into TODO list",
            contribution,
            level
        );
        self.list.write().push(TodoItem {
            contribution,
            level,
        });
    }

    pub fn get_best(&self) -> Option<(C, usize, usize)> {
        let list = self.list.upgradable_read();

        let mut best_i = 0;
        let mut best_score = list.first()?.evaluate(Arc::clone(&self.evaluator));

        for (i, todo) in list.iter().enumerate().skip(1) {
            let score = todo.evaluate(Arc::clone(&self.evaluator));
            if score > best_score {
                best_i = i;
                best_score = score;
            }
        }

        //trace!("Best score: {}", best_score);
        if best_score > 0 {
            let mut list = RwLockUpgradableReadGuard::upgrade(list);
            let best_todo = list.swap_remove(best_i);
            Some((best_todo.contribution, best_todo.level, best_score))
        } else {
            None
        }
    }
}
