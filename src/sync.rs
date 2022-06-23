// MIT/Apache2 License

#[cfg(not(loom))]
pub use alloc::sync::Arc;
#[cfg(not(loom))]
pub use core::sync::atomic::{AtomicUsize, Ordering::SeqCst};
#[cfg(loom)]
pub use loom::sync::{
    atomic::{AtomicUsize, Ordering::SeqCst},
    Arc,
};

#[cfg(feature = "std")]
mod std_sync {
    pub type Lazy<T> = once_cell::sync::Lazy<T>;
    pub type Mutex<T> = std::sync::Mutex<T>;

    pub fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        match mutex.lock() {
            Ok(guard) => guard,
            Err(err) => {
                tracing::error!("bypassing poison: {:?}", &err);
                err.into_inner()
            }
        }
    }
}

#[cfg(not(feature = "std"))]
mod spin_sync {
    pub type Lazy<T> = spin::Lazy<T>;
    pub type Mutex<T> = spin::Mutex<T>;

    pub fn lock<T>(mutex: &Mutex<T>) -> spin::MutexGuard<'_, T> {
        mutex.lock()
    }
}

#[cfg(not(feature = "std"))]
pub(crate) use spin_sync::*;
#[cfg(feature = "std")]
pub(crate) use std_sync::*;
