// MIT/Apache2 License

//! `breadthread` is an abstraction for what I've noticed is a very common pattern for low-level graphics APIs
//! to have:
//!
//! * One thread is designated the "GUI Thread". All GUI operations must occur in this thread.
//! * Primitives are thread-unsafe and can only be used safely in the GUI thread.
//! * In order to run the framework, an event loop function has to be called in a loop.
//! * This "main loop" produces events.
//!
//! In order to abstract over this model in a not just memory-safe, but thread-safe way, the `breadthread`
//! package:
//!
//! * Provides the [`BreadThread`] object. When this object is instantiated, the thread it's instantiated in
//!   becomes the "bread thread". The `BreadThread` is `!Send` and thus remains fixed to the bread thread.
//! * The `BreadThread` is provided an object implementing the [`Controller`] trait, which determines how it
//!   runs the main event loop.
//! * Objects implementing the [`Directive`] trait are used to send "messages" to the `Controller`, telling it
//!   what operations it should run.
//! * The `BreadThread` also creates "events". An "event handler" can be provided to the `BreadThread` in order
//!   to run something whenever an event is generated.
//! * While the `BreadThread` is `!Send`, it can create [`ThreadHandle`]s that can be sent to other threads.
//! * When the first `ThreadHandle` is created, the "directive thread" is spawned. This thread listens for events
//!   and then pushes them into the controller's directive queue.
//! * When the `ThreadHandle` is used from another thread, it sends its directives to the directive thread.
//!   However, if it is used from the bread thread, it forwards its directives straight to the controller.
//!
//! # Features
//!
//! `breadthread` uses mutexes internally. When the `pl` feature is enabled, it uses `parking_lot`
//! mutexes instead of `std::sync` mutexes. This may be useful in programs that already use `parking_lot`.

#![forbid(unsafe_code)]

mod bread_thread;
mod completer;
mod controller;
mod directive;
mod key;
mod thread_handle;
mod thread_state;

pub(crate) mod directive_thread;
pub(crate) mod mutex;

pub use bread_thread::*;
pub use completer::*;
pub use controller::*;
pub use directive::*;
pub use key::*;
pub use thread_handle::*;

pub(crate) use thread_state::*;

use std::{error::Error as StdError, fmt, num::NonZeroUsize};
use thread_safe::NotInOriginThread;

/// An error occurred during operation of `breadthread`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Error<InnerError> {
    /// Directive contained external pointers that didn't belong.
    InvalidPtr(NonZeroUsize),
    /// The bread thread has ceased operation.
    Closed,
    /// The controller failed to complete processing a directive.
    UnableToComplete,
    /// Attempted to turn a thread that was already a bread thread into a bread thread.
    AlreadyABreadThread,
    /// We are not in the bread thread.
    NotInBreadThread,
    /// A controller-associated error has occurred.
    Controller(InnerError),
}

impl<InnerError> From<NotInOriginThread> for Error<InnerError> {
    #[inline]
    fn from(_: NotInOriginThread) -> Error<InnerError> {
        Error::NotInBreadThread
    }
}

impl<InnerError: fmt::Display> fmt::Display for Error<InnerError> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidPtr(ptr) => {
                write!(f, "Directive contained external pointer that does not belong to the bread thread: {:#08X}", ptr)
            }
            Error::Closed => f.write_str("The bread thread is closed and cannot be used"),
            Error::UnableToComplete => f.write_str("The controller did not complete the directive"),
            Error::AlreadyABreadThread => f.write_str("This thread is already a bread thread"),
            Error::NotInBreadThread => f.write_str("This thread is not the bread thread"),
            Error::Controller(inner) => fmt::Display::fmt(inner, f),
        }
    }
}

impl<InnerError: fmt::Display + fmt::Debug> StdError for Error<InnerError> {
    #[inline]
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Error::NotInBreadThread => Some(&NotInOriginThread),
            _ => None,
        }
    }
}
