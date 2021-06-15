// MIT/Apache2 License

use super::{Controller, Error, ThreadHandle, ThreadState};
use orphan_crippler::Receiver;
use std::{any::Any, cell::Cell, marker::PhantomData, sync::Arc, thread_local};

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
    // ensure this object is !Send and !Sync
    _phantom: PhantomData<*const ThreadState<'evh, Ctrl>>,
}

impl<'evh, Ctrl: Controller> BreadThread<'evh, Ctrl> {
    /// Create a new `BreadThread` from a controller. This function returns an error if the thread is already a
    /// bread thread.
    #[inline]
    pub fn try_new(controller: Ctrl) -> Result<Self, Error<Ctrl::Error>> {
        if let Err(e) = IS_BREAD_THREAD.with(|ibt| {
            if ibt.replace(true) {
                Err(Error::AlreadyABreadThread)
            } else {
                Ok(())
            }
        }) {
            Err(e)
        } else {
            Ok(Self {
                state: Arc::new(ThreadState::new(controller)),
                _phantom: PhantomData,
            })
        }
    }

    /// Create a new `BreadThread` from a controller. This function panics an error if the thread is already a
    /// bread thread.
    #[inline]
    pub fn new(controller: Ctrl) -> Self {
        Self::try_new(controller).unwrap_or_else(|_| panic!("Thread is already a bread thread"))
    }

    /// Send a directive to the thread's controller.
    #[inline]
    pub fn send_directive<T: Any + Send>(
        &mut self,
        directive: Ctrl::Directive,
    ) -> Result<Receiver<T>, Error<Ctrl::Error>> {
        // SAFETY: since this is !send, we know we're in the bread thread
        unsafe {
            self.state
                .send_directive(directive, self.state.bread_thread_id())
        }
    }

    /// Set the event handler we are using for the bread thread.
    #[inline]
    pub fn set_event_handler<F: FnMut(&Ctrl, Ctrl::Event) + Send + 'evh>(
        &mut self,
        event_handler: F,
    ) {
        self.state.set_event_handler(event_handler);
    }

    /// Run the main loop.
    #[inline]
    pub fn main_loop(self) -> Result<(), Error<Ctrl::Error>> {
        // SAFETY: we are in the bread thread
        while unsafe { self.state.loop_cycle() }? {}
        Ok(())
    }

    /// Create a handle to this `BreadThread` that can send directives.
    #[inline]
    pub fn handle(&self) -> ThreadHandle<'evh, Ctrl> {
        self.state.init_directive_channel();
        ThreadHandle::from_weak(Arc::downgrade(&self.state))
    }
}

impl<'evh, Ctrl: Controller> Drop for BreadThread<'evh, Ctrl> {
    #[inline]
    fn drop(&mut self) {
        let _ = IS_BREAD_THREAD.try_with(|ibt| ibt.set(false));
    }
}
