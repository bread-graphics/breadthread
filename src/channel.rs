// MIT/Apache2 License

//! Channel types to be used within `breadthread` for sending/receiving data.
//!
//! In the common case (`std` is enabled), these are implemented using the
//! `flume` crate. In the future, we may move away from `flume` if other
//! options (like `crossbeam`) turn out to be better.
//!
//! In `no_std`, these are implemented by wrapping a `VecDeque` around a simple
//! spinlock. The `wait` method is unavailable in this case.

#[cfg(feature = "std")]
mod flume_channel {
    const FLUME_CHANNEL_LIMIT: usize = 1024;

    pub(crate) struct Sender<T>(flume::Sender<T>);
    pub(crate) struct Receiver<T>(flume::Receiver<T>);

    impl<T> Clone for Sender<T> {
        fn clone(&self) -> Self {
            Sender(self.0.clone())
        }
    }

    impl<T> Clone for Receiver<T> {
        fn clone(&self) -> Self {
            Receiver(self.0.clone())
        }
    }

    pub(crate) use flume::TrySendError;

    pub(crate) fn channel<T>() -> (Sender<T>, Receiver<T>) {
        // tell whether we should unbound the channel or not
        let unbound_channel = std::env::var_os("BREADTHREAD_UNBOUNDED_CHANNEL");

        let (s, r) = if unbound_channel.is_some() {
            // we need an unbounded channel
            flume::unbounded()
        } else {
            // we need a bounded channel
            flume::bounded(FLUME_CHANNEL_LIMIT)
        };

        (Sender(s), Receiver(r))
    }

    impl<T> Sender<T> {
        pub(crate) fn send(&self, value: T) -> Result<(), flume::TrySendError<T>> {
            self.0.try_send(value)
        }
    }

    impl<T> Receiver<T> {
        pub(crate) fn try_recv(&self) -> Result<T, flume::TryRecvError> {
            self.0.try_recv()
        }

        pub(crate) fn recv(&self) -> Result<T, flume::RecvError> {
            self.0.recv()
        }
    }
}

#[cfg(not(feature = "std"))]
mod spin_channel {
    use alloc::collections::VecDeque;

    #[cfg(not(loom))]
    use alloc::sync::Arc;
    #[cfg(not(loom))]
    use core::sync::atomic::{
        AtomicUsize,
        Ordering::{Relaxed, SeqCst},
    };
    #[cfg(loom)]
    use loom::sync::{
        atomic::{
            AtomicUsize,
            Ordering::{Relaxed, SeqCst},
        },
        Arc,
    };

    struct Channel<T> {
        queue: spin::Mutex<VecDeque<T>>,
        sender_count: AtomicUsize,
        receiver_count: AtomicUsize,
    }

    pub(crate) struct Sender<T>(Arc<Channel<T>>);
    pub(crate) struct Receiver<T>(Arc<Channel<T>>);

    pub(crate) fn channel<T>() -> (Sender<T>, Receiver<T>) {
        let channel = Channel {
            queue: spin::Mutex::new(VecDeque::new()),
            sender_count: AtomicUsize::new(1),
            receiver_count: AtomicUsize::new(1),
        };
        let channel = Arc::new(channel);

        (Sender(channel.clone()), Receiver(channel))
    }

    impl<T> Clone for Sender<T> {
        fn clone(&self) -> Self {
            self.0.sender_count.fetch_add(1, SeqCst);
            Sender(self.0.clone())
        }
    }

    impl<T> Clone for Receiver<T> {
        fn clone(&self) -> Self {
            self.0.receiver_count.fetch_add(1, SeqCst);
            Receiver(self.0.clone())
        }
    }

    impl<T> Drop for Sender<T> {
        fn drop(&mut self) {
            self.0.sender_count.fetch_sub(1, SeqCst);
        }
    }

    impl<T> Drop for Receiver<T> {
        fn drop(&mut self) {
            self.0.receiver_count.fetch_sub(1, SeqCst);
        }
    }

    pub(crate) enum TrySendError<T> {
        Full(T),
        Disconnected(T),
    }

    pub(crate) enum TryRecvError {
        Empty,
        Disconnected,
    }

    impl<T> Sender<T> {
        pub(crate) fn send(&self, value: T) -> Result<(), TrySendError<T>> {
            if self.0.receiver_count.load(Relaxed) == 0 {
                return Err(TrySendError::Disconnected(value));
            }

            let mut queue = self.0.queue.lock();
            queue.push_back(value);
            Ok(())
        }
    }

    impl<T> Receiver<T> {
        pub(crate) fn try_recv(&self) -> Result<T, TryRecvError> {
            if self.0.sender_count.load(Relaxed) == 0 {
                return Err(TryRecvError::Disconnected);
            }

            let mut queue = self.0.queue.lock();
            match queue.pop_front() {
                Some(value) => Ok(value),
                None => Err(TryRecvError::Empty),
            }
        }
    }
}

#[cfg(feature = "std")]
pub(crate) use flume_channel::*;
#[cfg(not(feature = "std"))]
pub(crate) use spin_channel::*;
