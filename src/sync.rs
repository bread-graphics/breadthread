// MIT/Apache2 License

#[cfg(feature = "std")]
mod std_sync {
    pub type Lazy<T> = once_cell::sync::Lazy<T>;
    pub type OnceCell<T> = once_cell::sync::OnceCell<T>;
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

    pub fn call_once<T>(cell: &OnceCell<T>, f: impl FnOnce() -> T) -> &T {
        cell.get_or_init(f)
    }

    pub fn oc_into_inner<T>(cell: OnceCell<T>) -> Option<T> {
        cell.into_inner()
    }

    pub fn oc_with_value<T>(val: T) -> OnceCell<T> {
        OnceCell::with_value(val)
    }
}

#[cfg(not(feature = "std"))]
mod spin_sync {
    pub type Lazy<T> = spin::Lazy<T>;
    pub type OnceCell<T> = spin::Once<T>;
    pub type Mutex<T> = spin::Mutex<T>;

    pub fn lock<T>(mutex: &Mutex<T>) -> spin::MutexGuard<'_, T> {
        mutex.lock()
    }

    pub fn call_once<T>(cell: &OnceCell<T>, f: impl FnOnce() -> T) -> &T {
        cell.call_once(f)
    }

    pub fn oc_into_inner<T>(cell: OnceCell<T>) -> Option<T> {
        cell.try_into_inner()
    }

    pub fn oc_with_value<T>(val: T) -> OnceCell<T> {
        let oc = OnceCell::new();
        oc.call_once(move || val);
        oc
    }
}

#[cfg(not(feature = "std"))]
pub(crate) use spin_sync::*;
#[cfg(feature = "std")]
pub(crate) use std_sync::*;
