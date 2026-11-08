//! `Locked`: lock a mutex even if another thread panicked while holding it.
//!
//! A panic while a lock is held "poisons" the mutex, and `lock()` then
//! returns an error. Splashdown's shared state stays usable after such a
//! panic (a half-finished update is better than a stuck app), so every lock
//! takes the value as it is. `m.locked()` says that in one word instead of
//! `m.lock().unwrap_or_else(|e| e.into_inner())`.

use std::sync::{Mutex, MutexGuard};

pub trait Locked<T: ?Sized> {
    /// Locks, ignoring poisoning.
    fn locked(&self) -> MutexGuard<'_, T>;
}

impl<T: ?Sized> Locked<T> for Mutex<T> {
    fn locked(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_poisoned_lock_still_gives_its_value() {
        let m = std::sync::Arc::new(Mutex::new(1));
        let held = m.clone();
        let _ = std::thread::spawn(move || {
            let _guard = held.lock().unwrap();
            panic!("poison it");
        })
        .join();
        assert!(m.lock().is_err());
        *m.locked() += 1;
        assert_eq!(*m.locked(), 2);
    }
}
