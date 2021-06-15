// MIT/Apache2 License

#[cfg(not(feature = "pl"))]
use std::sync;

#[cfg(not(feature = "pl"))]
pub(crate) struct Mutex<D> {
    inner: sync::Mutex<D>,
}

#[cfg(feature = "pl")]
pub(crate) struct Mutex<D> {
    inner: parking_lot::Mutex<D>,
}

impl<D> Mutex<D> {
    #[inline]
    pub(crate) fn new(data: D) -> Self {
        cfg_if::cfg_if! {
            if #[cfg(feature = "pl")] {
                Self { inner: parking_lot::Mutex::new(data) }
            } else {
                Self { inner: sync::Mutex::new(data) }
            }
        }
    }

    #[cfg(feature = "pl")]
    #[inline]
    pub(crate) fn lock(&self) -> parking_lot::MutexGuard<'_, D> {
        self.inner.lock()
    }

    #[cfg(not(feature = "pl"))]
    #[inline]
    pub(crate) fn lock(&self) -> sync::MutexGuard<'_, D> {
        self.inner.lock().expect("Unable to lock mutex")
    }
}

#[cfg(not(feature = "pl"))]
pub(crate) struct RwLock<D> {
    inner: sync::RwLock<D>,
}

#[cfg(feature = "pl")]
pub(crate) struct RwLock<D> {
    inner: parking_lot::RwLock<D>,
}

impl<D> RwLock<D> {
    #[inline]
    pub(crate) fn new(data: D) -> Self {
        cfg_if::cfg_if! {
            if #[cfg(feature = "pl")] {
                Self { inner: parking_lot::RwLock::new(data) }
            } else {
                Self { inner: sync::RwLock::new(data) }
            }
        }
    }

    #[cfg(feature = "pl")]
    #[inline]
    pub(crate) fn read(&self) -> parking_lot::RwLockReadGuard<'_, D> {
        self.inner.read()
    }

    #[cfg(feature = "pl")]
    #[inline]
    pub(crate) fn write(&self) -> parking_lot::RwLockWriteGuard<'_, D> {
        self.inner.write()
    }

    #[cfg(not(feature = "pl"))]
    #[inline]
    pub(crate) fn read(&self) -> sync::RwLockReadGuard<'_, D> {
        self.inner
            .read()
            .expect("Unable to lock rwlock for reading")
    }

    #[cfg(not(feature = "pl"))]
    #[inline]
    pub(crate) fn write(&self) -> sync::RwLockWriteGuard<'_, D> {
        self.inner
            .write()
            .expect("Unable to lock rwlock for writing")
    }
}
