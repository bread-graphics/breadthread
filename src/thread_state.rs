// MIT/Apache2 License

use crate::{
    directive_thread::{launch_directive_thread, DirectiveThreadMessage},
    mutex::RwLock,
    AddOrRemovePtr, Controller, DirectCompleter, Directive, Error, LoopCycle, SendCompleter,
};
use flume::{Receiver, Sender};
use once_cell::sync::{Lazy, OnceCell};
use orphan_crippler::{Receiver as OcReceiver, Sender as OcSender};
use std::{any::Any, cell::RefCell, collections::HashSet, num::NonZeroUsize};
use thread_safe::{ThreadKey, ThreadSafe};

/// The inner state that the `BreadThread` owns and the `ThreadHandle`s keep a reference to. This contains the
/// controller and generally dictates what each part of the program does.
pub(crate) struct ThreadState<'evh, Ctrl: Controller> {
    /// The controller used to dictate what the threads do. This should only be accessed directly by the thread
    /// in which the `BreadThread` sits. Therefore, we use a `RefCell` so that the owning thread will have
    /// mutable access to the controller. Without sifting through atomics. We also use a `ThreadSafe` so only our
    /// origin thread can ever access it.
    controller: ThreadSafe<RefCell<Ctrl>>,
    /// It is necessary to send messages to the directive thread, telling it whether or not it needs to close
    /// down. This acts as a way to send these messages to the directive thread. Note that the directive thread
    /// is only initialized once a `ThreadHandle` is explicitly created, so a `OnceCell` is used in order to
    /// express this. A sync `OnceCell` is used because it may be accessed from multiple threads.
    directive_thread_messenger: OnceCell<DirectiveThreadMessenger<Ctrl::Directive>>,
    /// The list of pointers that belongs to the `BreadThread`.
    pointers: RwLock<HashSet<NonZeroUsize>>,
    /// The current event handler.    
    event_handler: EventHandler<'evh, Ctrl>,
}

type BoxedEventHandler<'evh, Ctrl> = Box<
    dyn FnMut(&Ctrl, <Ctrl as Controller>::Event) -> Result<(), <Ctrl as Controller>::Error>
        + Send
        + 'evh,
>;

type SenderReceiverPair<'evh, Ctrl> = (
    Sender<BoxedEventHandler<'evh, Ctrl>>,
    Receiver<BoxedEventHandler<'evh, Ctrl>>,
);

/// The interface for handling events.
struct EventHandler<'evh, Ctrl: Controller> {
    /// The function itself. Kept in a `ThreadSafe<RefCell>` instead of a `Mutex`, since it's only read in the
    /// current thread and changing it is a cold operation.
    function: ThreadSafe<RefCell<BoxedEventHandler<'evh, Ctrl>>>,
    /// Sender and receiver for changing the event handler. These are lazily initialized.
    changer: Lazy<SenderReceiverPair<'evh, Ctrl>>,
}

/// The interface for messaging the directive thread.
struct DirectiveThreadMessenger<Dir> {
    /// Send messages to the directive thread.
    message: Sender<DirectiveThreadMessage>,
    /// The directive thread only acknowledges it has a message if `None` is sent along its directive channel.
    directive_channel: Sender<Option<OcSender<Dir>>>,
}

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
            controller: ThreadSafe::new(RefCell::new(controller)),
            directive_thread_messenger: OnceCell::new(),
            pointers: RwLock::new(HashSet::new()),
            event_handler: EventHandler {
                function: ThreadSafe::new(RefCell::new(Box::new(|_, _| Ok(())))),
                changer: Lazy::new(flume::unbounded),
            },
        }
    }

    /// Use the inner controller in a closure.
    ///
    /// # Errors
    ///
    /// Errors out if it isn't the bread thread.
    #[inline]
    pub(crate) fn with<T, F: FnOnce(&Ctrl) -> T>(
        &self,
        f: F,
        key: ThreadKey,
    ) -> Result<T, Error<Ctrl::Error>> {
        Ok(f(&self.controller.try_get_ref_with_key(key)?.borrow()))
    }

    /// Use the inner controller in a closure, mutably.
    ///
    /// # Errors
    ///
    /// Errors out if this isn't the bread thread.
    #[inline]
    pub(crate) fn with_mut<T, F: FnOnce(&mut Ctrl) -> T>(
        &self,
        f: F,
        key: ThreadKey,
    ) -> Result<T, Error<Ctrl::Error>> {
        Ok(f(&mut self
            .controller
            .try_get_ref_with_key(key)?
            .borrow_mut()))
    }

    /// Set the current event handler.
    #[inline]
    pub(crate) fn set_event_handler<
        Ev: FnMut(&Ctrl, Ctrl::Event) -> Result<(), Ctrl::Error> + Send + 'evh,
    >(
        &self,
        handler: Ev,
    ) {
        self.event_handler
            .changer
            .0
            .try_send(Box::new(handler))
            .expect("As long as this object lives, the channel should be intact")
    }

    /// Get the directive channel used to send directives to the bread thread.
    #[inline]
    fn directive_channel(&self) -> &DirectiveThreadMessenger<Ctrl::Directive> {
        &self
            .directive_thread_messenger
            .get_or_try_init(|| {
                // SAFETY: we're on the bread thread, we can safely access "controller"
                let adaptor = self
                    .controller
                    .try_get_ref()?
                    .borrow_mut()
                    .directive_adaptor();

                // start the thread
                let (message, directive_channel) = launch_directive_thread(adaptor);

                Result::<_, thread_safe::NotInOriginThread>::Ok(DirectiveThreadMessenger {
                    message,
                    directive_channel,
                })
            })
            .unwrap()
    }

    /// Ensure that the directive channel is initialized.
    #[inline]
    pub(crate) fn init_directive_channel(&self) {
        let _ = self.directive_channel();
    }

    /// Run a loop cycle. Returns false if the loop should stop.
    ///
    /// # Errors
    ///
    /// This function will error out if it is not the bread thread.
    #[inline]
    pub(crate) fn loop_cycle(&self, key: ThreadKey) -> Result<bool, Error<Ctrl::Error>> {
        let res = self
            .controller
            .try_get_ref_with_key(key)?
            .borrow_mut()
            .loop_cycle();
        match res {
            Err(e) => Err(Error::Controller(e)),
            Ok(LoopCycle::Continue(event)) => {
                // SAFETY: we are running in the bread thread
                self.process_event(event, key)?;
                Ok(true)
            }
            Ok(LoopCycle::Directive(mut sender)) => {
                let directive = sender.input().expect("Someone took the input directive");
                let mut completer = SendCompleter::new(sender);
                self.process_ptrs(
                    self.controller
                        .try_get_ref_with_key(key)?
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
    /// # Errors
    ///
    /// This errors out if this is not the bread thread.
    #[inline]
    pub(crate) fn send_directive<T: Any + Send>(
        &self,
        directive: Ctrl::Directive,
        key: ThreadKey,
    ) -> Result<OcReceiver<T>, Error<Ctrl::Error>> {
        self.validate_ptrs(&directive)?;

        if let Ok(controller) = self.controller.try_get_ref_with_key(key) {
            // we are on the bread thread, we can just use the directive thread
            let mut completer = DirectCompleter::Empty;
            self.process_ptrs(
                controller
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
            .into_iter()
            .filter(|ptr| !pointers.contains(ptr))
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
                    pointers.insert(p);
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
    /// # Errors
    ///
    /// This function errors out if it is not the bread thread.
    #[inline]
    pub(crate) fn process_event(
        &self,
        event: Ctrl::Event,
        key: ThreadKey,
    ) -> Result<(), Error<Ctrl::Error>> {
        let evhbox = self.event_handler.function.try_get_ref_with_key(key)?;
        // shouldn't panic, we're on the same thread
        let controller = self.controller.get_ref_with_key(key).borrow();
        if let Ok(mut event_handler) = self.event_handler.changer.1.try_recv() {
            let res = event_handler(&controller, event);
            *evhbox.borrow_mut() = event_handler;
            match res {
                Ok(res) => Ok(res),
                Err(e) => Err(Error::Controller(e)),
            }
        } else {
            match (evhbox.borrow_mut())(&controller, event) {
                Ok(res) => Ok(res),
                Err(e) => Err(Error::Controller(e)),
            }
        }
    }
}
