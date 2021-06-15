// MIT/Apache2 License

use core::{any::Any, num::NonZeroUsize};

/// Represents a directive that can be sent to a bread thread.
pub trait Directive: Any + Send {
    /// Get a list of pointers contained within this directive.
    fn pointers(&self) -> &[NonZeroUsize];
}
