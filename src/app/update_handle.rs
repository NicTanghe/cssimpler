use std::sync::{Arc, Mutex};

type Wake = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct PendingUpdate {
    requested: bool,
    wake: Option<Wake>,
}

/// Requests an application update from any thread.
///
/// Attach the handle with [`super::App::with_update_handle`]. Publish results
/// before calling [`Self::request_update`]. Requests arriving before the next
/// update are coalesced, and the application's update decides what to repaint.
#[derive(Clone, Default)]
pub struct UpdateHandle(Arc<Mutex<PendingUpdate>>);

impl UpdateHandle {
    pub fn request_update(&self) {
        let wake = {
            let mut pending = self.0.lock().expect("update handle poisoned");
            if pending.requested {
                return;
            }
            pending.requested = true;
            pending.wake.clone()
        };
        if let Some(wake) = wake {
            wake();
        }
    }

    pub(super) fn install_wake(&self, wake: Wake) {
        let requested = {
            let mut pending = self.0.lock().expect("update handle poisoned");
            pending.wake = Some(wake.clone());
            pending.requested
        };
        if requested {
            wake();
        }
    }

    pub(super) fn begin_update(&self) {
        self.0.lock().expect("update handle poisoned").requested = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn wakes_for_pending_work_and_once_per_update() {
        let handle = UpdateHandle::default();
        let wakes = Arc::new(AtomicUsize::new(0));
        let count = wakes.clone();
        handle.request_update();
        handle.install_wake(Arc::new(move || {
            count.fetch_add(1, Ordering::Relaxed);
        }));
        for _ in 0..10 {
            handle.request_update();
        }
        assert_eq!(wakes.load(Ordering::Relaxed), 1);
        handle.begin_update();
        handle.request_update();
        assert_eq!(wakes.load(Ordering::Relaxed), 2);
    }
}
