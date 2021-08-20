// MIT/Apache2 License

use super::{Completer, Directive};
use orphan_crippler::Sender;
use std::{fmt::Debug, num::NonZeroUsize};

/// This object dictates what the `BreadThread` will do, exactly.
pub trait Controller {
    /// The type representing a directive for this controller. Essentially, this represents something that tells
    /// the controller what to do.
    type Directive: Directive;
    /// The type representing a thread-safe handle to this controller's event mechanism.
    type DirectiveAdaptor: DirectiveAdaptor<Self::Directive> + Send + 'static;
    /// The error type of the loop cycle.
    type Error: Debug;
    /// The type representing an event that may be produced by the loop cycle.
    type Event;
    /// The collection type used to tell how we should add or remove pointers.
    ///
    /// If the iterator's size is not exact, it may cause panics down the line.
    type Pointers: IntoIterator<Item = AddOrRemovePtr>;

    /// Get a directive adaptor.
    fn directive_adaptor(&self) -> Self::DirectiveAdaptor;
    /// Run an event loop cycle. Do not process any directives during this time.
    fn loop_cycle(&self) -> Result<LoopCycle<Self::Event, Self::Directive>, Self::Error>;
    /// Process a directive and send the result down a `Sender`. Returns the list of pointers to add or remove
    /// from the verified pointers list.
    ///
    /// Since this may be used re-entrantly, it has to be immutable.
    fn process_directive<C: Completer>(
        &self,
        directive: Self::Directive,
        completer: &mut C,
    ) -> Self::Pointers;
}

/// Add or remove a pointer from the verified pointers list.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum AddOrRemovePtr {
    /// Do nothing. This only exists so this enum can have a `Default` impl.
    DoNothing,
    /// Add a pointer to the verified list.
    AddPtr(NonZeroUsize, usize),
    /// Remove a pointer from the verified list.
    RemovePtr(NonZeroUsize),
}

impl Default for AddOrRemovePtr {
    #[inline]
    fn default() -> Self {
        AddOrRemovePtr::DoNothing
    }
}

/// The result of a loop cycle.
pub enum LoopCycle<Event, Directive> {
    /// Stop the loop.
    Break,
    /// We got an event.
    Continue(Event),
    /// We got a directive.
    Directive(Sender<Directive>),
}

/// This object sends events to the main `Controller`.
pub trait DirectiveAdaptor<Directive> {
    /// Send a directive over this `DirectiveAdaptor`.
    fn send(&mut self, directive: Sender<Directive>);
}
