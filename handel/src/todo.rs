use std::sync::Arc;

use parking_lot::{RwLock, RwLockUpgradableReadGuard};

use crate::multisig::Signature;
use crate::evaluator::Evaluator;


#[derive(Clone, Debug)]
struct TodoItem {
    signature: Signature,
    level: usize,
}

impl TodoItem {
    pub fn evaluate<E: Evaluator>(&self, evaluator: Arc<E>) -> usize {
        evaluator.evaluate(&self.signature, self.level)
    }
}

#[derive(Debug)]
pub(crate) struct TodoList<E: Evaluator> {
    list: RwLock<Vec<TodoItem>>,
    evaluator: Arc<E>,
    //task: Option<Task>
}

impl<E: Evaluator> TodoList<E> {
    pub fn new(evaluator: Arc<E>) -> Self {
        Self {
            list: RwLock::new(Vec::new()),
            evaluator,
            //task: None,
        }
    }

    pub fn put(&self, signature: Signature, level: usize) {
        trace!("Putting {:?} (level {}) into TODO list", signature, level);
        self.list.write().push(TodoItem {
            signature,
            level
        });
    }

    pub fn get_best(&self) -> Option<(Signature, usize, usize)> {
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
            Some((best_todo.signature, best_todo.level, best_score))
        }
        else {
            None
        }
    }
}
