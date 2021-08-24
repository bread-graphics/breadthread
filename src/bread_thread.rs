// MIT/Apache2 License

use super::{AddOrRemovePtr, Controller, Error, ThreadHandle, ThreadState};
use orphan_crippler::Receiver;
use std::{any::Any, cell::Cell, sync::Arc, thread_local};
use thread_safe::ThreadKey;

thread_local! {
    /// There cannot be more than one `BreadThread` per thread. This indicates if this thread is currently a
    /// bread thread.
    static IS_BREAD_THREAD: Cell<bool> = Cell::new(false);
}

/// The object representing the bread thread. The thread this object is created in is the bread thread, and it
/// cannot be moved out of the bread thread.
///
/// The lifetime associated with this object is the lifetime of whatever event handler is passed in. If you want
/// to pass its handles between threads, this lifetime will probably be `'static` unless you're using scoped
/// threads or scoped executors.
pub struct BreadThread<'evh, Ctrl: Controller> {
    state: Arc<ThreadState<'evh, Ctrl>>,
    // key that represents that the thread we're on is the bread thread, also makes this `!Send`
    key: ThreadKey,
}

impl<'evh, Ctrl: Controller> BreadThread<'evh, Ctrl> {
    /// Create a new `BreadThread` from a controller. This function returns an error if the thread is already a
    /// bread thread.
    #[inline]
    pub fn try_new(controller: Ctrl) -> Result<Self, Error<Ctrl::Error>> {
        if IS_BREAD_THREAD.with(|ibt| ibt.replace(true)) {
            Err(Error::NotInBreadThread)
        } else {
            Ok(Self {
                state: Arc::new(ThreadState::new(controller)),
                key: ThreadKey::get(),
            })
        }
    }

    /// Create a new `BreadThread` from a controller. This function panics an error if the thread is already a
    /// bread thread.
    #[inline]
    pub fn new(controller: Ctrl) -> Self {
        Self::try_new(controller).unwrap_or_else(|_| panic!("Thread is already a bread thread"))
    }

    /// Use the controller in a closure.
    #[inline]
    pub fn with<T, F: FnOnce(&Ctrl) -> T>(&self, f: F) -> T {
        // we are guaranteed to be in the bread thread
        self.state.with(f, self.key).unwrap()
    }

    /// Use the controller in a mutable closure.
    #[inline]
    pub fn with_mut<T, F: FnOnce(&mut Ctrl) -> T>(&mut self, f: F) -> T {
        // we are guaranteed to be in the bread thread
        Arc::get_mut(&mut self.state)
            .expect("Already has handles")
            .with_mut(f, self.key)
            .unwrap()
    }

    /// Send a directive to the thread's controller.
    #[inline]
    pub fn send_directive<T: Any + Send>(
        &self,
        directive: Ctrl::Directive,
    ) -> Result<Receiver<T>, Error<Ctrl::Error>> {
        self.state.send_directive(directive, self.key)
    }

    /// Set the event handler we are using for the bread thread.
    #[inline]
    pub fn set_event_handler<
        F: FnMut(&Ctrl, Ctrl::Event) -> Result<(), Ctrl::Error> + Send + 'evh,
    >(
        &self,
        event_handler: F,
    ) {
        self.state.set_event_handler(event_handler);
    }

    /// Process an event using the currently set event handler.
    #[inline]
    pub fn process_event(&self, event: Ctrl::Event) -> Result<(), Error<Ctrl::Error>> {
        self.state.process_event(event, self.key)
    }

    /// Run the main loop.
    #[inline]
    pub fn main_loop(self) -> Result<(), Error<Ctrl::Error>> {
        while self.state.loop_cycle(self.key)? {}
        Ok(())
    }

    /// Create a handle to this `BreadThread` that can send directives.
    #[inline]
    pub fn handle(&self) -> ThreadHandle<'evh, Ctrl> {
        self.state.init_directive_channel();
        ThreadHandle::from_weak(Arc::downgrade(&self.state))
    }

    /// Manually process pointers.
    #[inline]
    pub fn process_ptrs<I: IntoIterator<Item = AddOrRemovePtr>>(&self, iter: I) {
        self.state.process_ptrs(iter)
    }
}

impl<'evh, Ctrl: Controller> Drop for BreadThread<'evh, Ctrl> {
    #[inline]
    fn drop(&mut self) {
        let _ = IS_BREAD_THREAD.try_with(|ibt| ibt.set(false));
    }
}
