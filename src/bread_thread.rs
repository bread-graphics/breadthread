// MIT/Apache2 License

use super::{Controller, Error, ThreadHandle, ThreadState};
use orphan_crippler::Receiver;
use std::{any::Any, marker::PhantomData, sync::Arc};

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
    /// Create a new `BreadThread` from a controller.
    #[inline]
    pub fn new(controller: Ctrl) -> Self {
        Self {
            state: Arc::new(ThreadState::new(controller)),
            _phantom: PhantomData,
        }
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
