// MIT/Apache2 License

//! A runtime that allows thread-unsafe code to be executed on a designated
//!
//! ## Motivation
//!
//! There are quite a few APIs that are thread unsafe. The one I had in mind
//! while designing this crate is the Windows `winuser` windowing API, but
//! there are many others. While small programs may be able to get away with
//! being thread unsafe, larger programs, with runtimes that require `Safe`
//! bounds, may not.
//!
//! Ordinarily, these programs will have to resort to convoluted systems to
//! ensure that code runs on a desigated "local" thread or thread pool. The
//! goal of this crate is to simplify these systems by providing a runtime
//! that allows thread-unsafe code to be executed on a designated thread.
//!
//! ## Usage
//!
//! First, create a type to use as a [`Tag`]. This type will be used to
//! uniquely identify the thread that the runtime will run on at compile time.
//! This ensures that values that are native to one thread will not be used
//! on another thread.
//!
//! Any `'static` type can be used as a tag, and it's recommended to use a
//! zero-sized type.
//!
//! ```rust
//! struct MyTag;
//! # let _ = MyTag;
//! ```
//!
//! Then, create a [`BreadThread`] type. This is the runtime that directives
//! are sent along. To spawn a new thread to run directives on, use the `new`
//! method.
//!
//! ```rust
//! use breadthread::BreadThread;
//!
//! # struct MyTag;
//! let bt = BreadThread::<'static, MyTag>::new();
//! # let _ = bt;
//! ```
//!
//! However, if you already have a system that you'd like to take advantage of
//! (a dedicated thread pool like [`rayon`], for instance), you can use the
//! `undriven()` method to create both a `BreadThread` and a [`Driver`]. You can
//! transform a thread or thread-like task into the driving thread by calling
//! `drive()` on the `Driver`.
//!
//! ```rust
//! # use breadthread::BreadThread;
//! # struct MyTag;
//! let (bt, driver) = BreadThread::<'static, MyTag>::undriven();
//! my_runtime::spawn_task(move || driver.drive());
//! # let _ = bt;
//! # mod my_runtime {
//! #     pub fn spawn_task<F: FnOnce()>(_f: F) {}
//! # }
//! ```
//!
//! Note that the `BreadThread` and `Driver` are parameterized by a lifetime, which
//! in this case is `'static`. The lifetime is used as a bound for the directives that
//! we send to the thread. Consider using this if you want to send a directive that borrows
//! other data.
//!
//! Now, we can call the `run` method on the `BreadThread` to run a given method.
//!
//! ```rust
//! # use breadthread::BreadThread;
//! # struct MyTag;
//! # let bt = BreadThread::<'static, MyTag>::new();
//!
//! use breadthread::DirectiveOutput;
//!
//! let input_value = 7;
//! let value = bt.run((), move |()| {
//!     let ret_ty = thread_unsafe_code(input_value);
//!     DirectiveOutput {
//!         thread_safe_value: (),
//!         thread_unsafe_value: ret_ty,
//!         deleted_values: vec![],
//!     }
//! });
//!
//! # fn thread_unsafe_code(_i: i32) -> i32 { 0 }
//! ```
//!
//! `bt.run()` expects a return type of [`DirectiveOutput`], which consists of:
//!
//! - A `thread_safe_value` that is implied to be `Send` and `Sync`.
//! - A `thread_unsafe_value` that may not be any of these. Once `value` returns, this
//!   value will be wrapped in [`Object`], which essentially allows it to be sent to
//!   other threads, but only used in the driving thread. To use this value again, pass it
//!   into the `bt.run()` method in place of the empty tuple, and it can be used raw
//!   again. Tuples and slices of `Object`s can be returned using this strategy.
//!   Note that values of this kind have to implement [`Compatible`].
//! - `deleted_values` consists of values to be deleted from the thread's internal bank
//!   that keeps track of the values that are valid for it. By default, values returned
//!   are added to the "valid" list, so if you don't expect to use the value again, you
//!   can add it to the `deleted_values` list.
//!
//! `value` is of type [`Value`], and resolves to a tuple of `thread_safe_value` and the
//! safe version of `thread_unsafe_value`. It can be resolved in one of three ways:
//!
//! - Poll for whether or not it's resolved using `value.resolve()`.
//! - Wait for it to be resolved by parking the thread, using `value.wait()`.
//! - With the `async` feature enabled, `Value` implements [`Future`].
//!
//! ## Safety
//!
//! Using `tag` ensures that the bread thread will only have values that are tagged as
//! valid associated with it. As long as two threads do not have the same tag, this
//! validation is done at compile time, and the only real overhead is in sending and
//! receiving values from the two threads.
//!
//! If more than one thread is created with the same tag, by default the library panics.
//! If this behavior is not desired, enable the `fallback` feature. Instead, when two
//! threads share a tag, they will manually keep track of which values are valid for
//! which thread.
//!
//! ## `no_std`
//!
//! The `std` feature is enabled by default. Without `std`, this crate only relies on the
//! `alloc` crate. However, certain changes are made both externally and internally.
//!
//! - `BreadThread::new()`, `Driv(er::drive()` and `Value::wait()` are not present.
//! - Internally, the data structures use spinlock-based APIs instead of ones based on
//!   system synchronization. This is often undesireable behavior.
//!
//! It is recommended to use the `std` feature unless it is necessary to use this crate
//! in a `no_std` environment.
//!
//! [`rayon`]: https://crates.io/crates/rayon
//! [`Future`]: std::future::Future

#![no_std]
#![deprecated = "It is probably a bad idea to use this crate"]

extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

use ahash::RandomState;
use alloc::boxed::Box;
use core::{any::TypeId, marker::PhantomData, mem};
use hashbrown::{hash_map::Entry, HashMap as HbHashMap, HashSet as HbHashSet};
use sync::{Arc, AtomicUsize, SeqCst};

pub use value::Value;

pub(crate) mod channel;
pub(crate) mod sync;
pub(crate) mod value;

mod current;
mod directive;
mod driver;
mod object;
mod wrapped;

pub use current::if_tagged_thread;
pub use directive::DirectiveOutput;
pub use driver::{Driver, Pinned, Unpinned};
pub use object::{Compatible, Object};
pub use wrapped::Wrapped;

/// A runtime for allowing thread unsafe code to be run on a designated
/// thread.
pub struct BreadThread<'lt, Tag: 'static> {
    // a channel used to send directives to the thread
    sender: channel::Sender<directive::Directive<'lt>>,
    // count to decrement on drop
    count: Arc<AtomicUsize>,
    _tag: PhantomData<Tag>,
}

impl<'lt, Tag: 'static> Drop for BreadThread<'lt, Tag> {
    fn drop(&mut self) {
        // decrement the count
        self.count.fetch_sub(1, SeqCst);
    }
}

impl<'lt, Tag: 'static> BreadThread<'lt, Tag> {
    /// Creates a new `BreadThread` along with a `Driver`.
    ///
    /// This method can be used to use the current thread as the driving thread,
    /// if that is desired.
    pub fn undriven() -> (Self, Driver<'lt, Unpinned>) {
        // check to see if we need to use fallback capabilities
        let id = TypeId::of::<Tag>();
        let fallback = match sync::lock(&*TAGS).entry(id) {
            Entry::Occupied(entry) => {
                let fallback = entry.get();
                fallback.fetch_add(1, SeqCst);
                fallback.clone()
            }
            Entry::Vacant(entry) => {
                let fallback = Arc::new(AtomicUsize::new(1));
                entry.insert(fallback.clone());
                fallback
            }
        };

        // if we don't have to deal with fallbacks, panic if we have more than one
        // thread with this tag active
        #[cfg(not(feature = "fallback"))]
        if fallback.load(SeqCst) > 1 {
            panic!(
                "
cannot create more than one BreadThread with the same tag
enable the `fallback` feature on the `breadthread` crate to allow this
            "
            );
        }

        let (sender, receiver) = channel::channel();
        let bt = Self {
            sender,
            count: fallback.clone(),
            _tag: PhantomData,
        };
        let driver = Driver::new::<Tag>(receiver, fallback);

        (bt, driver)
    }

    /// Send a new directive to the thread to be polled and used.
    pub fn run<
        Input: 'lt + Wrapped<Tag>,
        NtsOutput: 'lt + Send + Sync,
        TsOutput: 'lt + Wrapped<Tag>,
    >(
        &self,
        input: Input,
        op: impl FnOnce(Input::Unwrapped) -> DirectiveOutput<NtsOutput, TsOutput::Unwrapped>
            + Send
            + 'lt,
    ) -> Value<(NtsOutput, TsOutput)> {
        // create the directive and output value
        let (directive, value) = directive::Directive::new(input, op);

        // try to send the directive
        match self.sender.send(directive) {
            Ok(()) => value,
            Err(channel::TrySendError::Disconnected(_)) => {
                panic!("Driver has been dropped, cannot send directive")
            }
            Err(channel::TrySendError::Full(_)) => {
                panic!("{}", CHANNEL_FULL)
            }
        }
    }
}

#[cfg(feature = "std")]
impl<'lt, Tag: 'static> BreadThread<'lt, Tag> {
    /// Create a new `BreadThread` that spawns a new thread that is used
    /// to run the directives.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        static ID_GENERATOR: std::sync::atomic::AtomicUsize =
            std::sync::atomic::AtomicUsize::new(0);
        let id = ID_GENERATOR.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let (this, driver) = BreadThread::undriven();

        // box a function for driving the thread, and cast it to make it static
        // SAFETY: driver will always outlive the bread thread
        let driver: Box<dyn FnOnce() + Send + 'lt> = Box::new(move || driver.pin().drive());
        let driver: *mut (dyn FnOnce() + Send + 'lt) = Box::into_raw(driver);
        let driver: *mut (dyn FnOnce() + Send + 'static) = unsafe { mem::transmute(driver) };
        let driver: Box<dyn FnOnce() + Send + 'static> = unsafe { Box::from_raw(driver as *mut _) };

        std::thread::Builder::new()
            .name(std::format!("breadthread-{}", id))
            .spawn(driver)
            .expect("failed to spawn thread");

        this
    }
}

/// A hash set containing tags that currently exist.
static TAGS: sync::Lazy<sync::Mutex<HashMap<TypeId, Arc<AtomicUsize>>>> =
    sync::Lazy::new(|| sync::Mutex::new(HashMap::with_hasher(RandomState::default())));

type HashSet<K> = HbHashSet<K, RandomState>;
type HashMap<K, V> = HbHashMap<K, V, RandomState>;

// error messages

const CHANNEL_FULL: &str = "
The bounded channel used to send directives to the driving thread is full.

This is unlikely to happen, and should only really happen in one of three cases:

1). The driving thread did not call `Driver::drive()` or `Driver::tick()`.
2). The driving thread is hanging on a directive that is never resolved.
3). Other threads sent too many directives too fast, and the driving thread is unable
    to process all of them.

If resolving one of these three use cases is not possible, set the
`BREADTHREAD_UNBOUNDED_CHANNEL` environment variable, and the channel will be
unbounded.
";
