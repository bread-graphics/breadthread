// MIT/Apache2 License

use crate::sync::{self, Mutex};
use alloc::sync::Arc;
use core::{
    cell::UnsafeCell,
    marker::Unpin,
    mem::{self, MaybeUninit},
    ptr,
    task::Waker,
};

#[cfg(feature = "std")]
use parking::Unparker;

use core::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

#[cfg(not(loom))]
use core::sync::atomic::{AtomicBool, Ordering::SeqCst};
#[cfg(loom)]
use loom::sync::atomic::{AtomicBool, Ordering::SeqCst};

/// A value that may eventually resolve.
pub struct Value<T>(Arc<ValueInner<T>>);

/// A setter for a `Value`.
pub(crate) struct Setter<T>(Value<T>);

impl<T> Unpin for Value<T> {}

struct ValueInner<T> {
    // whether or not the value in "slot" is valid
    valid: AtomicBool,
    // the slot that contains the value
    slot: UnsafeCell<MaybeUninit<T>>,
    // the waiter waiting on this value
    waiter: Mutex<Waiter>,
}

// SAFETY: valid + slot makes up what's essentially a partially
// atomic `Option<T>` that never gives out references
unsafe impl<T: Send> Send for ValueInner<T> {}
unsafe impl<T: Send> Sync for ValueInner<T> {}

enum Waiter {
    None,
    #[cfg(feature = "std")]
    Unpark(Unparker),
    Waker(Waker),
}

impl<T> Value<T> {
    pub(crate) fn new() -> (Self, Setter<T>) {
        let inner = Arc::new(ValueInner {
            valid: AtomicBool::new(false),
            slot: UnsafeCell::new(MaybeUninit::uninit()),
            waiter: Mutex::new(Waiter::None),
        });

        (Self(inner.clone()), Setter(Self(inner)))
    }

    /// Create a `Value` that is already resolved.
    ///
    /// This is useful in code where once branch may need to use
    /// the driving thread, but another can be resolved immediately.
    pub fn immediate(val: T) -> Self {
        Value(Arc::new(ValueInner {
            valid: AtomicBool::new(true),
            slot: UnsafeCell::new(MaybeUninit::new(val)),
            waiter: Mutex::new(Waiter::None),
        }))
    }

    pub fn is_resolved(&self) -> bool {
        self.0.valid.load(SeqCst)
    }

    /// Convert to the inner value.
    ///
    /// # Safety
    ///
    /// This is unsafe because the value may not be resolved.
    unsafe fn take_inner(&mut self) -> T {
        debug_assert!(self.is_resolved());

        self.0.valid.store(false, SeqCst);
        ptr::read(self.0.slot.get() as *const T)
    }

    /// Try to resolve the value immediately.
    pub fn resolve(mut self) -> Result<T, Value<T>> {
        // tell if we can resolve yet
        if self.is_resolved() {
            Ok(unsafe { self.take_inner() })
        } else {
            Err(self)
        }
    }

    fn put_waiter(&mut self, waiter: Waiter) {
        sync::lock(&self.0.waiter).replace(waiter);
    }

    /// Wait for the value to be resolved.
    #[cfg(feature = "std")]
    pub fn wait(self) -> T {
        let mut this = match self.resolve() {
            Ok(val) => return val,
            Err(this) => this,
        };

        // wait for the value to be resolved
        let (parker, unparker) = parking::pair();
        this.put_waiter(Waiter::Unpark(unparker));

        match this.resolve() {
            Ok(val) => val,
            Err(mut this) => {
                // wait for the value to be resolved
                parker.park();

                // this will only occur if we've been resolved
                unsafe { this.take_inner() }
            }
        }
    }
}

impl<T> Future for Value<T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        if this.is_resolved() {
            return Poll::Ready(unsafe { this.take_inner() });
        }

        // set the atomic waker to our new waker
        this.put_waiter(Waiter::Waker(cx.waker().clone()));

        // try again
        if this.is_resolved() {
            Poll::Ready(unsafe { this.take_inner() })
        } else {
            Poll::Pending
        }
    }
}

impl<T> Setter<T> {
    fn inner(&self) -> &ValueInner<T> {
        &(self.0).0
    }

    /// Put in a value, and perhaps signaling a `Value` on the other end.
    pub(crate) fn put(mut self, value: T) {
        // SAFETY: it is impossible for another value to be put in here
        unsafe { ptr::write(self.inner().slot.get() as *mut T, value) };
        self.inner().valid.store(true, SeqCst);
        self.0.put_waiter(Waiter::None)
    }
}

impl Waiter {
    fn replace(&mut self, other: Waiter) {
        let this = mem::replace(self, other);

        match this {
            Self::None => {}
            Self::Waker(w) => w.wake(),
            #[cfg(feature = "std")]
            Self::Unpark(p) => {
                p.unpark();
            }
        }
    }
}

impl<T> Drop for ValueInner<T> {
    fn drop(&mut self) {
        // drop if the valid is valid
        if *self.valid.get_mut() {
            // drop the value
            unsafe {
                ptr::drop_in_place(self.slot.get() as *mut T);
            }
        }
    }
}
