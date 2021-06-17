// MIT/Apache2 License

use core::num::NonZeroUsize;

/// Represents a directive that can be sent to a bread thread.
pub trait Directive: Send + 'static {
    /// The type that contains the pointers.
    type Pointers: IntoIterator<Item = NonZeroUsize>;
    /// Get a list of pointers contained within this directive.
    fn pointers(&self) -> Self::Pointers;
}
