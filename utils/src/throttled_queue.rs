use nimiq_collections::{UniqueLinkedList, Queue};
use std::time::Duration;
use std::time::Instant;
use std::cmp;
use std::hash::Hash;

/// A `ThrottledQueue` is a `Queue` that only allows unique elements and
/// limits the number of elements that can be retrieved over a period of time.
/// It restricts the number of potentially available elements to an upper limit of `max_at_once`.
/// When retrieving an element, the number of available elements will be decreased.
/// At a given interval however, this number will be increased again by an `allowance`.
/// Moreover, the `ThrottledQueue` allows to limit its size.
pub struct ThrottledQueue<T>
    where T: Hash + Eq {
    queue: UniqueLinkedList<T>,
    max_size: Option<usize>,
    max_at_once: usize,
    allowance: usize,
    allowance_interval: Duration,
    last_allowance: Instant,
    available_now: usize,
}

impl<T> ThrottledQueue<T>
    where T: Hash + Eq {
    /// Creates an empty `ThrottledQueue`.
    ///
    /// * `max_at_once` - The total maximum of potentially available elements at any point in time.
    /// * `allowance_interval` - The interval at which the number of potentially available elements will be increased by `allowance_num` to at most `max_at_once`.
    /// * `allowance` - The allowance to increase the number of potentially available elements by at the given `allowance_interval`.
    /// * `max_size` - This parameter can be used to limit the size of the queue. When exceeding this number, elements will be dequeued.
    ///
    /// It will be enforced that `allowance` < `max_at_once`.
    #[inline]
    pub fn new(max_at_once: usize, allowance_interval: Duration, allowance: usize, max_size: Option<usize>) -> Self {
        ThrottledQueue {
            queue: UniqueLinkedList::new(),
            max_size,
            max_at_once,
            allowance: cmp::min(max_at_once, allowance),
            allowance_interval,
            last_allowance: Instant::now(),
            available_now: max_at_once,
        }
    }

    /// Determines the number of elements available in the `ThrottledQueue`.
    pub fn num_available(&mut self) -> usize {
        // Check for more allowance if interval is defined.
        if self.allowance_interval > Duration::default() {
            let now = Instant::now();
            let mut duration = now.duration_since(self.last_allowance);
            let mut num_intervals: usize = 0;
            // TODO: Improve performance as soon as `div_duration` is available.
            // let num_intervals = now.duration_since(self.last_allowance).div_duration(self.allowance_interval);
            while let Some(left) = duration.checked_sub(self.allowance_interval) {
                num_intervals += 1;
                duration = left;
            }
            if num_intervals > 0 {
                self.available_now = cmp::min(self.max_at_once, self.available_now + (num_intervals as usize) * self.allowance);
            }
            self.last_allowance = now;
        }
        cmp::min(self.available_now, self.len())
    }

    /// Determines whether there are elements available in the `ThrottledQueue`.
    pub fn is_available(&mut self) -> bool {
        self.num_available() > 0
    }

    /// Enqueues an element to the back of a queue if it is not yet present in the list.
    pub fn push_back(&mut self, elt: T) {
        if let Some(max_size) = self.max_size {
            if self.queue.len() >= max_size {
                self.queue.pop_front();
            }
        }

        self.queue.push_back(elt);
    }

    /// Removes the first element and returns it, or `None` if the list is
    /// empty.
    pub fn pop_front(&mut self) -> Option<T> {
        if self.num_available() > 0 {
            self.available_now -= 1;
            return self.queue.pop_front();
        }

        None
    }
}

impl<T> Queue<T> for ThrottledQueue<T>
    where T: Hash + Eq {
    fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    fn len(&self) -> usize {
        self.queue.len()
    }

    fn clear(&mut self) {
        self.queue.clear()
    }

    fn peek(&self) -> Option<&T> {
        // TODO: `peek` does not recompute number of available elements.
        if self.available_now > 0 {
            return self.queue.peek();
        }

        None
    }

    fn dequeue(&mut self) -> Option<T> {
        self.pop_front()
    }

    fn dequeue_multi(&mut self, n: usize) -> Vec<T> {
        let mut v = Vec::new();
        for _ in 0..n {
            match self.dequeue() {
                Some(elt) => v.push(elt),
                None => return v,
            }
        }
        v
    }

    fn enqueue(&mut self, elt: T) {
        self.push_back(elt)
    }

    fn append(&mut self, other: &mut Self) {
        // It is important not to throttle here.
        while let Some(elt) = other.queue.pop_front() {
            self.push_back(elt);
        }
    }

    fn requeue(&mut self, elt: T) {
        self.requeue(elt)
    }
}
