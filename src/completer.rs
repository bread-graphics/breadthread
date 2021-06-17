// MIT/Apache2 License

use orphan_crippler::Sender;
use std::any::Any;

/// Represents an object that is used to complete a directive process.
pub trait Completer {
    /// Complete this object.
    fn complete<T: Any + Send>(&mut self, object: T);
}

/// Just contains the value directly.
pub(crate) enum DirectCompleter {
    Complete(Box<dyn Any + Send>),
    Empty,
}

impl Completer for DirectCompleter {
    #[inline]
    fn complete<T: Any + Send>(&mut self, object: T) {
        match self {
            Self::Complete(_) => panic!("Already completed"),
            t => {
                *t = Self::Complete(Box::new(object));
            }
        }
    }
}

/// Sends the value across a channel.
pub(crate) struct SendCompleter<Dir> {
    inner: Option<Sender<Dir>>,
}

impl<Dir> SendCompleter<Dir> {
    #[inline]
    pub(crate) fn new(inner: Sender<Dir>) -> Self {
        Self { inner: Some(inner) }
    }

    #[inline]
    pub(crate) fn completed(&self) -> bool {
        self.inner.is_none()
    }
}

impl<Dir> Completer for SendCompleter<Dir> {
    #[inline]
    fn complete<T: Any + Send>(&mut self, object: T) {
        match self.inner.take() {
            None => panic!("Already completed"),
            Some(inner) => inner.send(object),
        }
    }
}
