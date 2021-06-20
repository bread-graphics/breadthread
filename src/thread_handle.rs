// MIT/Apache2 License

use super::{BreadThread, Controller, Error, ThreadState};
use orphan_crippler::Receiver;
use std::{
    any::Any,
    sync::{mpsc, Arc, Weak},
    thread,
};
use thread_safe::ThreadKey;

/// A handle to the bread thread that can be sent between threads.
#[repr(transparent)]
pub struct ThreadHandle<'evh, Ctrl: Controller> {
    state: Weak<ThreadState<'evh, Ctrl>>,
}

impl<'evh, Ctrl: Controller> Clone for ThreadHandle<'evh, Ctrl> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
        }
    }
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
        state.send_directive(directive, ThreadKey::get())
    }

    /// Set the event handler for the thread.
    #[inline]
    pub fn set_event_handler<
        F: FnMut(&Ctrl, Ctrl::Event) -> Result<(), Ctrl::Error> + Send + 'evh,
    >(
        &self,
        event_handler: F,
    ) -> Result<(), Error<Ctrl::Error>> {
        let state = self.state()?;
        state.set_event_handler(event_handler);
        Ok(())
    }

    /// Process an event using the currently set event handler.
    ///
    /// # Errors
    ///
    /// This errors out if not run on the bread thread.
    #[inline]
    pub fn process_event(&self, event: Ctrl::Event) -> Result<(), Error<Ctrl::Error>> {
        let state = self.state()?;
        state.process_event(event, ThreadKey::get())
    }

    /// Use a closure with the controller, if we are on the bread thread.
    #[inline]
    pub fn with<T, F: FnOnce(&Ctrl) -> T>(&self, f: F) -> Result<T, Error<Ctrl::Error>> {
        let state = self.state()?;
        state.with(f, ThreadKey::get())
    }

    /// Pin this thread handle to a thread.
    #[inline]
    pub fn pin(self) -> PinnedThreadHandle<'evh, Ctrl> {
        PinnedThreadHandle {
            inner: self,
            key: ThreadKey::get(),
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
    key: ThreadKey,
}

impl<'evh, Ctrl: Controller> Clone for PinnedThreadHandle<'evh, Ctrl> {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            key: self.key,
        }
    }
}

impl<'evh, Ctrl: Controller> PinnedThreadHandle<'evh, Ctrl> {
    /// Send a directive to the thread.
    #[inline]
    pub fn send_directive<T: Any + Send>(
        &self,
        directive: Ctrl::Directive,
    ) -> Result<Receiver<T>, Error<Ctrl::Error>> {
        let state = self.inner.state()?;
        state.send_directive(directive, self.key)
    }

    /// Set the event handler for the thread.
    #[inline]
    pub fn set_event_handler<
        F: FnMut(&Ctrl, Ctrl::Event) -> Result<(), Ctrl::Error> + Send + 'evh,
    >(
        &self,
        event_handler: F,
    ) -> Result<(), Error<Ctrl::Error>> {
        self.inner.set_event_handler(event_handler)
    }

    /// Process an event using the currently set event handler. This errors out if this is not pinned to the
    /// bread thread.
    #[inline]
    pub fn process_event(&self, event: Ctrl::Event) -> Result<(), Error<Ctrl::Error>> {
        let state = self.inner.state()?;
        state.process_event(event, self.key)
    }

    /// Use a closure with the controller, if we are on the bread thread.
    #[inline]
    pub fn with<T, F: FnOnce(&Ctrl) -> T>(&self, f: F) -> Result<T, Error<Ctrl::Error>> {
        let state = self.inner.state()?;
        state.with(f, self.key)
    }

    /// Unpin this thread handle.
    #[inline]
    pub fn into_inner(self) -> ThreadHandle<'evh, Ctrl> {
        self.inner
    }
}
