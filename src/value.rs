// MIT/Apache2 License

use crate::sync;
use alloc::sync::Arc;
use core::marker::Unpin;

#[cfg(feature = "std")]
use parking::Unparker;

#[cfg(feature = "async")]
use atomic_waker::AtomicWaker;
#[cfg(feature = "async")]
use core::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

/// A value that may eventually resolve.
pub struct Value<T>(Option<Arc<ValueInner<T>>>);

impl<T> Unpin for Value<T> {}

struct ValueInner<T> {
    slot: sync::OnceCell<T>,

    /// Unparker used to park/unpark thread for
    /// synchronous waiting.
    #[cfg(feature = "std")]
    unparker: sync::OnceCell<Unparker>,
    /// Waker used to wake task for async waiting.
    #[cfg(feature = "async")]
    waker: AtomicWaker,
}

impl<T> Value<T> {
    pub(crate) fn new() -> Self {
        Value(Some(Arc::new(ValueInner {
            slot: sync::OnceCell::new(),
            #[cfg(feature = "std")]
            unparker: sync::OnceCell::new(),
            #[cfg(feature = "async")]
            waker: AtomicWaker::new(),
        })))
    }

    pub(crate) fn clone(&self) -> Self {
        Value(self.0.clone())
    }

    pub(crate) fn store(&self, val: T) {
        sync::call_once(&self.inner().slot, move || val);

        // wake up any waiters if we need to
        #[cfg(feature = "std")]
        if let Some(unparker) = self.inner().unparker.get() {
            unparker.unpark();
        }
    }

    fn inner(&self) -> &Arc<ValueInner<T>> {
        self.0.as_ref().expect("polled value after completion")
    }

    /// Create a `Value` that is already resolved.
    ///
    /// This is useful in code where once branch may need to use
    /// the driving thread, but another can be resolved immediately.
    pub fn immediate(val: T) -> Self {
        Value(Some(Arc::new(ValueInner {
            slot: sync::oc_with_value(val),
            #[cfg(feature = "std")]
            unparker: sync::OnceCell::new(),
            #[cfg(feature = "async")]
            waker: AtomicWaker::new(),
        })))
    }

    pub fn is_resolved(&self) -> bool {
        self.inner().slot.get().is_some() && Arc::strong_count(self.inner()) == 1
    }

    /// Convert to the inner value, panic if not ready.
    fn take_inner(&mut self) -> T {
        let slot = Arc::try_unwrap(self.0.take().expect("value polled after completion"))
            .unwrap_or_else(|_| panic!("Value is still held onto by other task"))
            .slot;
        sync::oc_into_inner(slot).expect("Value is not yet ready")
    }

    /// Try to resolve the value immediately.
    pub fn resolve(mut self) -> Result<T, Value<T>> {
        // tell if we can resolve yet
        if self.is_resolved() {
            Ok(self.take_inner())
        } else {
            Err(self)
        }
    }

    /// Wait for the value to be resolved.
    #[cfg(feature = "async")]
    pub fn wait(self) -> T {
        let this = match self.resolve() {
            Ok(val) => return val,
            Err(this) => this,
        };

        // wait for the value to be resolved
        let (parker, unparker) = parking::pair();
        sync::call_once(&this.inner().unparker, move || unparker);

        match this.resolve() {
            Ok(val) => val,
            Err(mut this) => {
                // wait for the value to be resolved
                parker.park();

                // this will only occur if we've been resolved
                this.take_inner()
            }
        }
    }
}

#[cfg(feature = "async")]
impl<T> Future for Value<T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        if this.is_resolved() {
            return Poll::Ready(this.take_inner());
        }

        // set the atomic waker to our new waker
        this.inner().waker.register(cx.waker());

        // try again
        if this.is_resolved() {
            Poll::Ready(this.take_inner())
        } else {
            Poll::Pending
        }
    }
}
