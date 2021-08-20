// MIT/Apache2 License

use core::num::NonZeroUsize;

/// Represents a directive that can be sent to a bread thread.
pub trait Directive: Send + 'static {
    /// The type that contains the pointers, along with a `usize` that uniquely identifies the type of the
    /// pointer.
    type Pointers: IntoIterator<Item = (NonZeroUsize, usize)>;
    /// Get a list of pointers contained within this directive.
    fn pointers(&self) -> Self::Pointers;
}
