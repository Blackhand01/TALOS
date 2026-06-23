use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static NEXT_LEASE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Uuid(u128);

impl Uuid {
    fn new_v4() -> Self {
        Self(u128::from(NEXT_LEASE_ID.fetch_add(1, Ordering::Relaxed)))
    }
}

impl std::fmt::Display for Uuid {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{:032x}", self.0)
    }
}

#[derive(Debug, Default)]
struct LeaseState {
    active: bool,
    active_id: Option<Uuid>,
}

#[derive(Clone, Debug, Default)]
pub struct GpuLeaseManager {
    state: Arc<Mutex<LeaseState>>,
}

#[derive(Debug)]
pub struct GpuLease {
    pub id: Uuid,
    state: Arc<Mutex<LeaseState>>,
}

impl GpuLeaseManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn try_acquire(&self) -> Option<GpuLease> {
        let mut state = self.state.lock().expect("lease mutex poisoned");
        if state.active {
            return None;
        }
        let id = Uuid::new_v4();
        state.active = true;
        state.active_id = Some(id);
        Some(GpuLease {
            id,
            state: Arc::clone(&self.state),
        })
    }

    pub fn is_active(&self) -> bool {
        self.state.lock().expect("lease mutex poisoned").active
    }
}

impl Drop for GpuLease {
    fn drop(&mut self) {
        let mut state = self.state.lock().expect("lease mutex poisoned");
        if state.active_id == Some(self.id) {
            state.active = false;
            state.active_id = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lease_is_exclusive_and_released_on_drop() {
        let manager = GpuLeaseManager::new();
        let lease = manager.try_acquire().expect("first lease should acquire");
        assert!(manager.is_active());
        assert!(manager.try_acquire().is_none());
        drop(lease);
        assert!(!manager.is_active());
        assert!(manager.try_acquire().is_some());
    }
}
