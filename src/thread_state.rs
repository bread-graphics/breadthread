// MIT/Apache2 License

use crate::{
    directive_thread::{launch_directive_thread, DirectiveThreadMessage},
    mutex::{Mutex, RwLock},
    AddOrRemovePtr, Controller, DirectCompleter, Directive, Error, LoopCycle, SendCompleter,
};
use flume::Sender;
use once_cell::sync::OnceCell;
use orphan_crippler::{Receiver as OcReceiver, Sender as OcSender};
use std::{
    any::Any,
    cell::RefCell,
    num::NonZeroUsize,
    thread::{self, ThreadId},
};

/// The inner state that the `BreadThread` owns and the `ThreadHandle`s keep a reference to. This contains the
/// controller and generally dictates what each part of the program does.
pub(crate) struct ThreadState<'evh, Ctrl: Controller> {
    /// The controller used to dictate what the threads do. This should only be accessed directly by the thread
    /// in which the `BreadThread` sits. Therefore, we use a `RefCell` so that the owning thread will have
    /// mutable access to the controller. Without sifting through atomics. This is semantically correct, I'll
    /// prove it later.
    controller: RefCell<Ctrl>,
    /// The thread ID that the `BreadThread` is in. This is used to check incoming requests in order to ensure
    /// whether or not they originate from the same thread.
    bread_thread_id: ThreadId,
    /// It is necessary to send messages to the directive thread, telling it whether or not it needs to close
    /// down. This acts as a way to send these messages to the directive thread. Note that the directive thread
    /// is only initialized once a `ThreadHandle` is explicitly created, so a `OnceCell` is used in order to
    /// express this. A sync `OnceCell` is used because it may be accessed from multiple threads.
    directive_thread_messenger: OnceCell<DirectiveThreadMessenger<Ctrl::Directive>>,
    /// The list of pointers that belongs to the `BreadThread`.
    pointers: RwLock<Vec<NonZeroUsize>>,
    /// The current event handler. The same guarantees backing the `RefCell` for the controller above back the
    /// `RefCell` here.
    #[allow(clippy::type_complexity)]
    event_handler: Mutex<Box<dyn FnMut(&Ctrl, Ctrl::Event) + Send + 'evh>>,
}

/// The interface for messaging the directive thread.
struct DirectiveThreadMessenger<Dir> {
    /// Send messages to the directive thread.
    message: Sender<DirectiveThreadMessage>,
    /// The directive thread only acknowledges it has a message if `None` is sent along its directive channel.
    directive_channel: Sender<Option<OcSender<Dir>>>,
}

// SAFETY: RefCell<Ctrl> may be `Send`, but it is not `Sync`. However, semantically, it is only ever used from
//         the bread thread. This is checked later on through use of the `bread_thread_id` value.
unsafe impl<'evh, Ctrl: Controller> Send for ThreadState<'evh, Ctrl> {}
unsafe impl<'evh, Ctrl: Controller> Sync for ThreadState<'evh, Ctrl> {}

impl<'evh, Ctrl: Controller> Drop for ThreadState<'evh, Ctrl> {
    #[inline]
    fn drop(&mut self) {
        if let Some(dtm) = self.directive_thread_messenger.get() {
            let _ = dtm.message.try_send(DirectiveThreadMessage::Stop);
        }
    }
}

impl<'evh, Ctrl: Controller> ThreadState<'evh, Ctrl> {
    /// Create a new `ThreadState` using a controller.
    #[inline]
    pub(crate) fn new(controller: Ctrl) -> Self {
        ThreadState {
            controller: RefCell::new(controller),
            // Assumes we are running in the same thread that the BreadThread is being instantiated in
            bread_thread_id: thread::current().id(),
            directive_thread_messenger: OnceCell::new(),
            pointers: RwLock::new(vec![]),
            event_handler: Mutex::new(Box::new(|_, _| ())),
        }
    }

    /// Get the bread thread's ID.
    #[inline]
    pub(crate) fn bread_thread_id(&self) -> ThreadId {
        self.bread_thread_id
    }

    /// Set the current event handler.
    #[inline]
    pub(crate) fn set_event_handler<Ev: FnMut(&Ctrl, Ctrl::Event) + Send + 'evh>(
        &self,
        handler: Ev,
    ) {
        *self.event_handler.lock() = Box::new(handler);
    }

    /// Get the directive channel used to send directives to the bread thread.
    #[inline]
    fn directive_channel(&self) -> &DirectiveThreadMessenger<Ctrl::Directive> {
        #[derive(Debug)]
        struct NotBreadThread;

        &self
            .directive_thread_messenger
            .get_or_try_init(|| {
                // check the current thread ID to ensure we're on the bread thread
                if thread::current().id() != self.bread_thread_id() {
                    return Err(NotBreadThread);
                }

                // SAFETY: we're on the bread thread, we can safely access "controller"
                let adaptor = self.controller.borrow_mut().directive_adaptor();

                // start the thread
                let (message, directive_channel) = launch_directive_thread(adaptor);

                Ok(DirectiveThreadMessenger {
                    message,
                    directive_channel,
                })
            })
            .unwrap()
    }

    /// Ensure that the directive channel is initialized.
    #[inline]
    pub(crate) fn init_directive_channel(&self) {
        self.directive_channel();
    }

    /// Run a loop cycle. Returns false if the loop should stop.
    ///
    /// # Safety
    ///
    /// This function assumes that it is running in the bread thread.
    #[inline]
    pub(crate) unsafe fn loop_cycle(&self) -> Result<bool, Error<Ctrl::Error>> {
        let res = self.controller.borrow_mut().loop_cycle();
        match res {
            Err(e) => Err(e.into()),
            Ok(LoopCycle::Continue(event)) => {
                // SAFETY: we are running in the bread thread
                self.process_event(event);
                Ok(true)
            }
            Ok(LoopCycle::Directive(mut sender)) => {
                let directive = sender.input().expect("Someone took the input directive");
                let mut completer = SendCompleter::new(sender);
                self.process_ptrs(
                    self.controller
                        .borrow()
                        .process_directive(directive, &mut completer),
                );

                if completer.completed() {
                    Ok(true)
                } else {
                    Err(Error::UnableToComplete)
                }
            }
            Ok(LoopCycle::Break) => Ok(false),
        }
    }

    /// Send a directive to the bread thread. This should just run the appropriate function if we are on the
    /// `BreadThread`.
    ///
    /// # Safety
    ///
    /// This function assumes that the `thread_id` parameter is the thread ID of the calling thread.
    #[inline]
    pub(crate) unsafe fn send_directive<T: Any + Send>(
        &self,
        directive: Ctrl::Directive,
        thread_id: ThreadId,
    ) -> Result<OcReceiver<T>, Error<Ctrl::Error>> {
        self.validate_ptrs(&directive)?;

        if self.bread_thread_id() == thread_id {
            // we are on the bread thread, we can just use the directive thread
            let mut completer = DirectCompleter::Empty;
            self.process_ptrs(
                self.controller
                    .borrow()
                    .process_directive(directive, &mut completer),
            );
            match completer {
                DirectCompleter::Complete(value) => {
                    match Box::<dyn Any + Send + 'static>::downcast(value) {
                        Ok(value) => Ok(orphan_crippler::complete_boxed(value)),
                        Err(_) => panic!("Value was of the wrong type!"),
                    }
                }
                DirectCompleter::Empty => Err(Error::UnableToComplete),
            }
        } else {
            // we are not on bread thread, push the directive into the directive channel
            let (directive, recv) = orphan_crippler::two(directive);
            self.directive_channel()
                .directive_channel
                .try_send(Some(directive))
                .expect("Channel should not fail");
            Ok(recv)
        }
    }

    /// Verify if some pointers are currently valid.
    #[inline]
    fn validate_ptrs(&self, directive: &Ctrl::Directive) -> Result<(), Error<Ctrl::Error>> {
        let pointers = self.pointers.read();
        let invalid: Vec<_> = directive
            .pointers()
            .iter()
            .filter(|ptr| !pointers.contains(ptr))
            .copied()
            .collect();
        if invalid.is_empty() {
            Ok(())
        } else {
            Err(Error::InvalidPtrs(invalid))
        }
    }

    /// Process adding and removing pointers.
    #[inline]
    fn process_ptrs<I: IntoIterator<Item = AddOrRemovePtr>>(&self, ptrs: I) {
        let ptrs = ptrs.into_iter();
        let (lo, hi) = ptrs.size_hint();
        assert_eq!(Some(lo), hi);

        if lo == 0 {
            return;
        }

        let mut blacklist = vec![];
        let mut pointers = self.pointers.write();
        for ptr in ptrs {
            match ptr {
                AddOrRemovePtr::DoNothing => (),
                AddOrRemovePtr::AddPtr(p) => {
                    pointers.push(p);
                }
                AddOrRemovePtr::RemovePtr(p) => {
                    blacklist.push(p);
                }
            }
        }

        pointers.retain(move |p| !blacklist.contains(&p));
    }

    /// Process an event.
    ///
    /// # Safety
    ///
    /// This function assumes that it is running on the bread thread.
    #[inline]
    unsafe fn process_event(&self, event: Ctrl::Event) {
        // SAFETY: we are running on the bread thread
        (self.event_handler.lock())(&self.controller.borrow(), event);
    }
}
