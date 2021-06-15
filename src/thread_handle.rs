// MIT/Apache2 License

use super::{Controller, Error, ThreadState};
use orphan_crippler::Receiver;
use std::{
    any::Any,
    marker::PhantomData,
    sync::{Arc, Weak},
    thread::{self, ThreadId},
};

/// A handle to the bread thread that can be sent between threads.
pub struct ThreadHandle<'evh, Ctrl: Controller> {
    state: Weak<ThreadState<'evh, Ctrl>>,
}

impl<'evh, Ctrl: Controller> ThreadHandle<'evh, Ctrl> {
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

    /// Unpin this thread handle.
    #[inline]
    pub fn into_inner(self) -> ThreadHandle<'evh, Ctrl> {
        self.inner
    }
}
