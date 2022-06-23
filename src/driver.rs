// MIT/Apache2 License

use crate::{
    channel,
    directive::Directive,
    sync::{Arc, AtomicUsize, SeqCst},
    HashSet,
};
use core::{any::TypeId, fmt, marker::PhantomData};

/// A driver that takes directives from a `BreadThread` and then runs them.
pub struct Driver<'lt, Status: PinStatus> {
    // a channel to receive directives from the bread thread
    receiver: channel::Receiver<Directive<'lt>>,
    // whether or not fallback mode is enabled
    fallback: Arc<AtomicUsize>,
    // list of items we can keep track of
    known_values: HashSet<usize>,
    // the tag that we're using
    tag: TypeId,
    // current status of being pinned
    marker: PhantomData<Status>,
}

impl<'lt, S: PinStatus> fmt::Debug for Driver<'lt, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Driver")
            .field("pending_messages", &self.receiver.pending())
            .field("fallback", &self.is_fallback())
            .field("pinned", &S::is_pinned())
            .finish()
    }
}

impl<'lt, S: PinStatus> Driver<'lt, S> {
    pub(crate) fn is_fallback(&self) -> bool {
        self.fallback.load(SeqCst) > 1
    }

    pub(crate) fn known_values(&self) -> &HashSet<usize> {
        &self.known_values
    }

    pub(crate) fn known_values_mut(&mut self) -> &mut HashSet<usize> {
        &mut self.known_values
    }
}

impl<'lt> Driver<'lt, Unpinned> {
    /// Pin this driver to the given thread.
    pub fn pin(self) -> Driver<'lt, Pinned> {
        let Self {
            receiver,
            fallback,
            known_values,
            tag,
            ..
        } = self;

        // set up tag id for current thread mechanism
        crate::current::set_thread_id(tag);

        Driver {
            receiver,
            fallback,
            known_values,
            tag,
            marker: PhantomData,
        }
    }

    pub(crate) fn new<Tag: 'static>(
        receiver: channel::Receiver<Directive<'lt>>,
        fallback: Arc<AtomicUsize>,
    ) -> Self {
        Self {
            receiver,
            fallback,
            known_values: HashSet::default(),
            tag: TypeId::of::<Tag>(),
            marker: PhantomData,
        }
    }
}

impl<'lt> Driver<'lt, Pinned> {
    /// Run a single task for this driver.
    pub fn tick(&mut self) {
        if let Ok(directive) = self.receiver.try_recv() {
            directive.run(self);
        }
    }
}

#[cfg(feature = "std")]
impl<'lt> Driver<'lt, Pinned> {
    /// Drive this driver until the other end is dropped.
    pub fn drive(mut self) {
        while let Ok(directive) = self.receiver.recv() {
            directive.run(&mut self);
        }
    }
}

/// This driver is not yet pinned to a particular thread.
pub struct Unpinned(());

/// This driver is pinned to a specific thread.
pub struct Pinned(*const ());

#[doc(hidden)]
pub trait PinStatus {
    fn is_pinned() -> bool;
}

impl PinStatus for Unpinned {
    fn is_pinned() -> bool {
        false
    }
}

impl PinStatus for Pinned {
    fn is_pinned() -> bool {
        true
    }
}
