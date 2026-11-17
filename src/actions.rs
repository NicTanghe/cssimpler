//! Typed, FIFO UI actions shared by event callbacks and an application update.
use std::sync::{Arc, Mutex};

type Wake = Arc<dyn Fn() + Send + Sync>;

struct Inbox<M> {
    pending: Vec<M>,
    wake: Option<Wake>,
}

/// A cloneable sender for typed UI actions. Cloning does not require `M: Clone`.
///
/// Actions are preserved in enqueue order, including duplicates. Storage grows as
/// needed; this unbounded queue is intended for UI commands, not streaming data.
/// Two buffers are reused by `App::with_actions` after their initial allocations.
pub struct ActionQueue<M> {
    inbox: Arc<Mutex<Inbox<M>>>,
}

impl<M> Clone for ActionQueue<M> {
    fn clone(&self) -> Self {
        Self {
            inbox: Arc::clone(&self.inbox),
        }
    }
}

impl<M> Default for ActionQueue<M> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M> ActionQueue<M> {
    pub fn new() -> Self {
        Self {
            inbox: Arc::new(Mutex::new(Inbox {
                pending: Vec::new(),
                wake: None,
            })),
        }
    }

    /// Enqueue an action and wake the attached runtime on an empty-to-nonempty
    /// transition. Wake callbacks run outside the queue lock.
    pub fn push(&self, action: M) {
        let wake = {
            let mut inbox = self.inbox.lock().expect("action queue poisoned");
            let was_empty = inbox.pending.is_empty();
            inbox.pending.push(action);
            if was_empty { inbox.wake.clone() } else { None }
        };
        if let Some(wake) = wake {
            wake();
        }
    }

    pub fn is_empty(&self) -> bool {
        self.inbox
            .lock()
            .expect("action queue poisoned")
            .pending
            .is_empty()
    }

    /// Replace an empty consumer buffer with the pending batch, retaining both
    /// allocations. Process the returned batch outside the lock, then clear it
    /// before the next call. Actions submitted meanwhile belong to the next batch.
    pub fn drain_into(&self, buffer: &mut Vec<M>) {
        assert!(buffer.is_empty(), "consumer buffer must be empty");
        let mut inbox = self.inbox.lock().expect("action queue poisoned");
        std::mem::swap(&mut inbox.pending, buffer);
    }
}

impl<M: Send + 'static> ActionQueue<M> {
    /// Build a reusable no-argument handler for `onclick_shared`.
    pub fn emit(&self, action: M) -> impl Fn() + Send + Sync + 'static
    where
        M: Clone + Sync,
    {
        let queue = self.clone();
        move || queue.push(action.clone())
    }

    /// Build a handler whose argument is converted into an action (for input
    /// values and other payloads). This does not require `M: Clone` or `M: Sync`.
    pub fn emit_with<T>(
        &self,
        map: impl Fn(T) -> M + Send + Sync + 'static,
    ) -> impl Fn(T) + Send + Sync + 'static {
        let queue = self.clone();
        move |value| queue.push(map(value))
    }
}

pub(crate) trait ActionSource: Send + Sync {
    fn has_pending(&self) -> bool;
    fn install_wake(&self, wake: Wake);
}

impl<M: Send + 'static> ActionSource for ActionQueue<M> {
    fn has_pending(&self) -> bool {
        !self.is_empty()
    }
    fn install_wake(&self, wake: Wake) {
        let pending = {
            let mut inbox = self.inbox.lock().expect("action queue poisoned");
            inbox.wake = Some(wake.clone());
            !inbox.pending.is_empty()
        };
        if pending {
            wake();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, Invalidation};
    use crate::renderer::{FrameInfo, SceneProvider};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    #[test]
    fn fifo_duplicates_payloads_and_next_batch() {
        let queue = ActionQueue::new();
        let click = queue.emit(String::from("click"));
        click();
        click();
        queue.emit_with(|s: &str| s.to_owned())("payload");
        let mut batch = Vec::new();
        queue.drain_into(&mut batch);
        queue.push("later".into());
        assert_eq!(batch, ["click", "click", "payload"]);
        batch.clear();
        queue.drain_into(&mut batch);
        assert_eq!(batch, ["later"]);
    }

    #[test]
    fn buffers_reuse_capacity_after_warmup() {
        let queue = ActionQueue::new();
        let mut batch = Vec::new();
        let mut pointers = Vec::new();
        for _ in 0..8 {
            queue.push(42);
            queue.drain_into(&mut batch);
            pointers.push(batch.as_ptr());
            batch.clear();
        }
        for i in 2..8 {
            assert_eq!(pointers[i], pointers[i - 2]);
        }
    }

    #[test]
    fn wake_is_coalesced_and_runs_outside_lock() {
        let queue = ActionQueue::new();
        queue.push(0);
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let weak = Arc::downgrade(&queue.inbox);
        let count = calls.clone();
        queue.install_wake(Arc::new(move || {
            // try_lock proves the producer released its mutex before waking.
            assert!(weak.upgrade().unwrap().try_lock().is_ok());
            count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }));
        queue.push(1);
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
        let mut batch = Vec::new();
        queue.drain_into(&mut batch);
        queue.push(2);
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 2);
    }

    #[test]
    fn concurrent_producers_preserve_every_action_and_each_producers_order() {
        // No Clone bound on messages, including when cloning the queue.
        struct Message(usize, usize);
        let queue = ActionQueue::new();
        std::thread::scope(|scope| {
            for producer in 0..4 {
                let sender = queue.clone();
                scope.spawn(move || {
                    for index in 0..100 {
                        sender.push(Message(producer, index));
                    }
                });
            }
        });
        let mut batch = Vec::new();
        queue.drain_into(&mut batch);
        let mut next = [0; 4];
        for Message(producer, index) in batch {
            assert_eq!(index, next[producer]);
            next[producer] += 1;
        }
        assert_eq!(next, [100; 4]);
    }

    #[test]
    fn app_processes_actions_refreshes_and_defers_reentrant_actions() {
        let sheet = crate::style::parse_stylesheet("").unwrap();
        let sender = Rc::new(RefCell::new(None::<ActionQueue<i32>>));
        let update_sender = sender.clone();
        let view_sender = sender.clone();
        let mut app = App::with_actions(
            Vec::<i32>::new(),
            &sheet,
            move |state, actions: &[i32], _| {
                state.extend_from_slice(actions);
                if actions == [1, 1, 2] {
                    update_sender.borrow().as_ref().unwrap().push(3);
                }
                Invalidation::Clean
            },
            move |state, queue| {
                *view_sender.borrow_mut() = Some(queue.clone());
                crate::ui! { <div>{format!("{state:?}")}</div> }
            },
        );
        let frame = |frame_index| FrameInfo {
            frame_index,
            delta: Duration::ZERO,
        };
        app.frame(frame(0));
        assert!(!app.needs_redraw());
        let queue = sender.borrow().as_ref().unwrap().clone();
        let wakes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = wakes.clone();
        SceneProvider::install_wake(
            &mut app,
            Arc::new(move || {
                counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }),
        );
        queue.push(1);
        queue.push(1);
        queue.push(2);
        assert!(app.needs_redraw());
        assert_eq!(wakes.load(std::sync::atomic::Ordering::Relaxed), 1);
        app.frame(frame(1));
        assert_eq!(app.state(), &[1, 1, 2]);
        assert!(app.latest_stats().rerendered);
        assert!(app.needs_redraw());
        app.frame(frame(2));
        assert_eq!(app.state(), &[1, 1, 2, 3]);
        assert!(!app.needs_redraw());
        app.frame(frame(3));
        assert!(!app.latest_stats().rerendered);
    }
}
