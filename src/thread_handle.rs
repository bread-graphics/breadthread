// MIT/Apache2 License

use super::{BreadThread, Controller, Error, ThreadState};
use orphan_crippler::Receiver;
use std::{
    any::Any,
    marker::PhantomData,
    sync::{mpsc, Arc, Weak},
    thread::{self, ThreadId},
};

/// A handle to the bread thread that can be sent between threads.
#[derive(Clone)]
#[repr(transparent)]
pub struct ThreadHandle<'evh, Ctrl: Controller> {
    state: Weak<ThreadState<'evh, Ctrl>>,
}

impl<'evh, Ctrl: Controller> ThreadHandle<'evh, Ctrl> {
    /// The `ThreadHandle` is represented internally as a pointer. This converts the `ThreadHandle` to that
    /// pointer. In order to safely deallocate the memory, this pointer needs to be converted back into a
    /// `ThreadHandle` using the [`from_ptr`] function.
    #[must_use = "If the pointer isn't put into from_raw, memory may not properly deallocate"]
    #[inline]
    pub fn into_raw(self) -> *const () {
        self.state.into_raw().cast()
    }

    /// Converts a raw pointer into a `ThreadHandle`.
    /// 
    /// # Safety
    /// 
    /// If this pointer was not created via the [`into_raw`] function, undefined behavior may occur.
    #[inline]
    pub unsafe fn from_raw(ptr: *const ()) -> Self {
        Self { state: Weak::from_raw(ptr.cast()) }
    }

    #[inline]
    fn state(&self) -> Result<Arc<ThreadState<'evh, Ctrl>>, Error<Ctrl::Error>> {
        match self.state.upgrade() {
            Some(state) => Ok(state),
            None => Err(Error::Closed),
        }
    }

    #[inline]
    pub(crate) fn from_weak(state: Weak<ThreadState<'evh, Ctrl>>) -> Self {
        Self { state }
    }

    /// Send a directive to the thread.
    #[inline]
    pub fn send_directive<T: Any + Send>(
        &self,
        directive: Ctrl::Directive,
    ) -> Result<Receiver<T>, Error<Ctrl::Error>> {
        let state = self.state()?;
        // SAFETY: thread::current().id() is the ID for the current thread
        unsafe { state.send_directive(directive, thread::current().id()) }
    }

    /// Set the event handler for the thread.
    #[inline]
    pub fn set_event_handler<F: FnMut(&Ctrl, Ctrl::Event) + Send + 'evh>(
        &self,
        event_handler: F,
    ) -> Result<(), Error<Ctrl::Error>> {
        let state = self.state()?;
        state.set_event_handler(event_handler);
        Ok(())
    }

    /// Use a closure with the controller, if we are on the bread thread.
    #[inline]
    pub fn with<T, F: FnOnce(&Ctrl) -> T>(&self, f: F) -> Result<T, Error<Ctrl::Error>> {
        let state = self.state()?;
        if thread::current().id() == state.bread_thread_id() {
            // SAFETY: we are on the bread thread
            Ok(unsafe { state.with(f) })
        } else {
            Err(Error::NotInBreadThread)
        }
    } 

    /// Pin this thread handle to a thread.
    #[inline]
    pub fn pin(self) -> PinnedThreadHandle<'evh, Ctrl> {
        PinnedThreadHandle {
            inner: self,
            thread_id: thread::current().id(),
            _phantom: PhantomData,
        }
    }
}

impl<Ctrl: Controller + Send + 'static> ThreadHandle<'static, Ctrl> {
    /// Initialize a new `BreadThread` in newly spawned thread, then clone a handle to that thread.
    #[inline]
    pub fn in_foreign_thread(controller: Ctrl) -> ThreadHandle<'static, Ctrl> {
        // initialize a channel to send the thread handle back from the thread
        let (sender, receiver) = mpsc::channel::<ThreadHandle<'static, Ctrl>>();
        thread::Builder::new()
            .name("bread-thread".to_string())
            .spawn(move || {
                let bt = BreadThread::new(controller);
                let th = bt.handle();
                sender
                    .send(th)
                    .expect("Receiver shouldn't have closed down already");
                bt.main_loop()
                    .unwrap_or_else(|_| panic!("Main loop failed"));
            })
            .expect("Unable to create foreign thread");
        receiver
            .recv()
            .expect("Sender shouldn't close down without sending")
    }
}

/// A handle to the bread thread that is locked to its thread. This allows it to omit a call to
/// `thread::current().id()` which saves time.
pub struct PinnedThreadHandle<'evh, Ctrl: Controller> {
    inner: ThreadHandle<'evh, Ctrl>,
    thread_id: ThreadId,
    // ensure this is !Send and !Sync
    _phantom: PhantomData<*const ThreadState<'evh, Ctrl>>,
}

impl<'evh, Ctrl: Controller> PinnedThreadHandle<'evh, Ctrl> {
    /// Send a directive to the thread.
    #[inline]
    pub fn send_directive<T: Any + Send>(
        &self,
        directive: Ctrl::Directive,
    ) -> Result<Receiver<T>, Error<Ctrl::Error>> {
        let state = self.inner.state()?;
        // SAFETY: since this can't be moved, thread_id will always be current
        unsafe { state.send_directive(directive, self.thread_id) }
    }

    /// Set the event handler for the thread.
    #[inline]
    pub fn set_event_handler<F: FnMut(&Ctrl, Ctrl::Event) + Send + 'evh>(
        &self,
        event_handler: F,
    ) -> Result<(), Error<Ctrl::Error>> {
        self.inner.set_event_handler(event_handler)
    }

    /// Use a closure with the controller, if we are on the bread thread.
    #[inline]
    pub fn with<T, F: FnOnce(&Ctrl) -> T>(&self, f: F) -> Result<T, Error<Ctrl::Error>> {
        let state = self.inner.state()?;
        if self.thread_id == state.bread_thread_id() {
            // SAFETY: we are on the bread thread
            Ok(unsafe { state.with(f) })
        } else {
            Err(Error::NotInBreadThread)
        }
    } 

    /// Unpin this thread handle.
    #[inline]
    pub fn into_inner(self) -> ThreadHandle<'evh, Ctrl> {
        self.inner
    }
}
