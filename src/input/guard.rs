use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct SyntheticInputGuard {
    inner: Arc<Mutex<GuardState>>,
    grace_window: Duration,
    watchdog_timeout: Duration,
}

#[derive(Debug)]
pub struct SyntheticInputGuardToken {
    inner: Arc<Mutex<GuardState>>,
    operation_id: u64,
    grace_window: Duration,
}

#[derive(Debug, Clone)]
struct GuardState {
    active_operation_id: Option<u64>,
    active_since: Option<Instant>,
    suppress_until: Option<Instant>,
    next_operation_id: u64,
}

impl Default for SyntheticInputGuard {
    fn default() -> Self {
        Self::new(Duration::from_millis(250), Duration::from_secs(10))
    }
}

impl SyntheticInputGuard {
    pub fn new(grace_window: Duration, watchdog_timeout: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(GuardState {
                active_operation_id: None,
                active_since: None,
                suppress_until: None,
                next_operation_id: 1,
            })),
            grace_window,
            watchdog_timeout,
        }
    }

    pub fn acquire(&self, now: Instant) -> SyntheticInputGuardToken {
        let mut state = self.inner.lock().expect("synthetic input guard poisoned");
        let operation_id = state.next_operation_id;
        state.next_operation_id += 1;
        state.active_operation_id = Some(operation_id);
        state.active_since = Some(now);
        state.suppress_until = None;

        SyntheticInputGuardToken {
            inner: Arc::clone(&self.inner),
            operation_id,
            grace_window: self.grace_window,
        }
    }

    pub fn is_suppressed(&self, now: Instant) -> bool {
        let mut state = self.inner.lock().expect("synthetic input guard poisoned");
        if let (Some(active_since), Some(_)) = (state.active_since, state.active_operation_id) {
            if now.saturating_duration_since(active_since) > self.watchdog_timeout {
                state.active_operation_id = None;
                state.active_since = None;
                state.suppress_until = Some(now + self.grace_window);
            }
        }

        state.active_operation_id.is_some()
            || state.suppress_until.is_some_and(|until| now <= until)
    }

    pub fn grace_window(&self) -> Duration {
        self.grace_window
    }
}

impl Drop for SyntheticInputGuardToken {
    fn drop(&mut self) {
        let now = Instant::now();
        let mut state = self.inner.lock().expect("synthetic input guard poisoned");
        if state.active_operation_id == Some(self.operation_id) {
            state.active_operation_id = None;
            state.active_since = None;
            state.suppress_until = Some(now + self.grace_window);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suppresses_while_token_is_active() {
        let guard = SyntheticInputGuard::default();
        let now = Instant::now();
        let token = guard.acquire(now);

        assert!(guard.is_suppressed(now + Duration::from_millis(1)));
        drop(token);
    }

    #[test]
    fn watchdog_clears_leaked_active_guard() {
        let guard = SyntheticInputGuard::new(Duration::from_millis(250), Duration::from_secs(1));
        let now = Instant::now();
        let token = guard.acquire(now);
        std::mem::forget(token);

        assert!(guard.is_suppressed(now + Duration::from_secs(2)));
    }

    #[test]
    fn future_active_since_does_not_panic() {
        let guard = SyntheticInputGuard::default();
        let now = Instant::now();
        let token = guard.acquire(now + Duration::from_millis(1));

        assert!(guard.is_suppressed(now));
        drop(token);
    }
}
