// MIT/Apache2 License

//! A runtime that allows thread-unsafe code to be executed on a designated
//! thread.

#![no_std]

extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

use ahash::RandomState;
use alloc::{boxed::Box, vec::Vec};
use core::{
    any::TypeId,
    marker::PhantomData,
    mem,
};
use hashbrown::{hash_map::Entry, HashMap as HbHashMap, HashSet as HbHashSet};

#[cfg(not(loom))]
use alloc::sync::Arc;
#[cfg(not(loom))]
use core::sync::atomic::{AtomicUsize, Ordering::SeqCst};
#[cfg(loom)]
use loom::sync::{
    atomic::{AtomicUsize, Ordering::SeqCst},
    Arc,
};

mod channel;
mod sync;

/// A runtime for allowing thread unsafe code to be run on a designated
/// thread.
pub struct BreadThread<'lt, Tag> {
    // a channel used to send directives to the thread
    sender: channel::Sender<Directive<'lt>>,
    _tag: PhantomData<Tag>,
}

/// A driver that takes directives from a `BreadThread` and then runs them.
pub struct Driver<'lt> {
    // a channel to receive directives from the bread thread
    receiver: channel::Receiver<Directive<'lt>>,
    // whether or not fallback mode is enabled
    fallback: Arc<AtomicUsize>,
    // list of items we can keep track of
    known_values: HashSet<usize>,
}

impl<'lt, Tag: 'static> BreadThread<'lt, Tag> {
    /// Creates a new `BreadThread` along with a `Driver`.
    ///
    /// This method can be used to use the current thread as the driving thread,
    /// if that is desired.
    pub fn undriven() -> (Self, Driver<'lt>) {
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
            _tag: PhantomData,
        };
        let driver = Driver {
            receiver,
            fallback,
            known_values: HbHashSet::default(),
        };

        (bt, driver)
    }

    /// Send a new directive to the thread to be polled and used.
    pub fn send<
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
        let (directive, value) = Directive::new(input, op);

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
        let driver: Box<dyn FnOnce() + Send + 'lt> = Box::new(move || driver.drive());
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

impl<'lt> Driver<'lt> {
    fn is_fallback(&self) -> bool {
        self.fallback.load(SeqCst) > 1
    }

    fn known_values(&self) -> &HashSet<usize> {
        &self.known_values
    }

    fn known_values_mut(&mut self) -> &mut HashSet<usize> {
        &mut self.known_values
    }

    /// Run a single task for this driver.
    pub fn tick(&mut self) {
        if let Ok(directive) = self.receiver.try_recv() {
            directive.run(self);
        }
    }
}

#[cfg(feature = "std")]
impl<'lt> Driver<'lt> {
    /// Drive this driver until the other end is dropped.
    pub fn drive(mut self) {
        while let Ok(directive) = self.receiver.recv() {
            directive.run(&mut self);
        }
    }
}

/// A directive to run a thread-unsafe operation using thread-unsafe primitives.
pub(crate) struct Directive<'lt> {
    // this is actually a wrapper around a closure that does everything we
    // need to for now
    closure: Box<dyn FnOnce(&mut Driver<'lt>) + Send + 'lt>,
}

impl<'lt> Directive<'lt> {
    /// Creates a new directive.
    pub(crate) fn new<
        Tag,
        Input: 'lt + Wrapped<Tag>,
        NtsOutput: 'lt + Send + Sync,
        TsOutput: 'lt + Wrapped<Tag>,
    >(
        input: Input,
        op: impl FnOnce(Input::Unwrapped) -> DirectiveOutput<NtsOutput, TsOutput::Unwrapped>
            + Send
            + 'lt,
    ) -> (Directive<'lt>, Value<(NtsOutput, TsOutput)>) {
        // create two value slots
        let ret_slot = Value::new();
        let input_slot = ret_slot.clone();

        let closure = Box::new(move |driver: &mut Driver<'lt>| {
            // first, ensure that, if we're in fallback mode, that we know of all of
            // the values we're going to need
            #[cfg(feature = "fallback")]
            if driver.is_fallback() {
                let known = driver.known_values();

                // SAFETY: we're on the thread
                unsafe {
                    input.for_each_representative(|value| {
                        if !known.contains(&value) {
                            panic!("unknown value passed to driver: {:X}", value);
                        }
                    });
                }
            }

            // unwrap the input
            // SAFETY: this will be executed on the given thread
            let unwrapped = unsafe { input.unwrap() };

            // run the op
            let DirectiveOutput {
                nts,
                ts,
                deleted_values,
            } = op(unwrapped);

            // wrap the out
            let ts_out = unsafe { TsOutput::wrap(ts) };

            // delete any values that were deleted, and add any values that are added
            #[cfg(feature = "fallback")]
            {
                let known = driver.known_values_mut();

                deleted_values.into_iter().for_each(|value| {
                    known.remove(&value);
                });

                // SAFETY: we're on the thread
                unsafe {
                    ts_out.for_each_representative(|value| {
                        known.insert(value);
                    });
                }
            }

            // store the output in the return slot
            input_slot.store((nts, ts_out));
        });

        (Self { closure }, ret_slot)
    }

    pub(crate) fn run(self, driver: &mut Driver<'lt>) {
        (self.closure)(driver);
    }
}

pub struct DirectiveOutput<NtsOutput, TsOutput> {
    /// The output value that is thread safe
    pub nts: NtsOutput,
    /// The output value that is thread unsafe
    pub ts: TsOutput,
    /// The list of values that can be removed from the driver's known values
    pub deleted_values: Vec<usize>,
}

/// A value that may eventually resolve.
pub struct Value<T>(Arc<ValueInner<T>>);

struct ValueInner<T> {
    slot: sync::OnceCell<T>,
    // TODO: wakers for async
}

impl<T> Value<T> {
    fn new() -> Self {
        Value(Arc::new(ValueInner {
            slot: sync::OnceCell::new(),
        }))
    }

    fn clone(&self) -> Self {
        Value(self.0.clone())
    }

    fn store(&self, val: T) {
        sync::call_once(&self.0.slot, move || val);
    }

    pub fn is_resolved(&self) -> bool {
        self.0.slot.get().is_some() && Arc::strong_count(&self.0) == 1
    }

    pub fn resolve(self) -> Result<T, Value<T>> {
        // tell if we can resolve yet
        if self.is_resolved() {
            let slot = Arc::try_unwrap(self.0)
                .unwrap_or_else(|_| panic!("Value is still held onto by other task"))
                .slot;
            Ok(sync::oc_into_inner(slot).unwrap())
        } else {
            Err(self)
        }
    }
}

/// A thread-unsafe object that only has true meaning in the context of a
/// `BreadThread` runtime.
#[repr(transparent)]
pub struct Object<Ty: ?Sized, Tag> {
    // the tag
    tag: PhantomData<Tag>,
    // the real object
    //
    // this will never drop, since it's in the Compatible contract
    // to never drop
    object: Ty,
}

// SAFETY: this is safe because the object is only ever accessed from its
// owning thread.
// double check sync though
unsafe impl<Ty, Tag> Send for Object<Ty, Tag> {}
unsafe impl<Ty, Tag> Sync for Object<Ty, Tag> {}

impl<Ty, Tag> Object<Ty, Tag> {
    /// Create a new `Object` without checking to see if
    /// we are on the correct thread.
    /// 
    /// # Safety
    /// 
    /// This is unsafe because it assumes that the caller is on the correct
    /// thread.
    pub unsafe fn new_unchecked(object: Ty) -> Self {
        Self {
            object,
            tag: PhantomData,
        }
    }

    /// Convert to the inner `Object` type.
    /// 
    /// # Safety
    /// 
    /// This is unsafe because it assumes that the caller is on the correct
    /// thread.
    pub unsafe fn into_inner_unchecked(self) -> Ty {
        self.object
    }
}

impl<Ty: ?Sized, Tag> Object<Ty, Tag> {
    /// Get a reference to the inner object.
    /// 
    /// # Safety
    /// 
    /// This is unsafe because it assumes that the caller is on the correct
    /// thread.
    pub unsafe fn get_unchecked(&self) -> &Ty {
        &self.object
    }
}

/// A thread-unsafe object that can be used in the `BreadThread` runtime.
///
/// # Safety
///
/// `representative` must be unique for the given object, and the type must not
/// have a `Drop` implementation of any kind.
pub unsafe trait Compatible {
    /// Get the representative value for this object.
    ///
    /// # Safety
    ///
    /// The representative object must be unique for this object.
    fn representative(&self) -> usize;
}

// SAFETY: copy indicates that this is a bitwise copy, meaning that
// malicious users can't fudge with the clone impl
// TODO: is Into<usize> a problem?
unsafe impl<T: Copy + Into<usize>> Compatible for T {
    fn representative(&self) -> usize {
        (*self).into()
    }
}

/// Wrap all of the levels of a tuple into `Compatible` types.
///
/// # Safety
///
/// Can only be implemented on a container of `Object<Ty, Tag>` types.
/// `Unwrapped` must be a container of the corresponding `Ty`.
pub unsafe trait Wrapped<Tag>: Send + Sync + Sized {
    type Unwrapped;

    /// Unwrap this object into the inner objects.
    /// 
    /// # Safety
    /// 
    /// This is unsafe because it assumes that the caller is on the correct
    /// thread.
    unsafe fn unwrap(self) -> Self::Unwrapped;
    /// Wrap the inner objects into this object.
    /// 
    /// # Safety
    /// 
    /// This is unsafe because it assumes that the caller is on the correct
    /// thread.
    unsafe fn wrap(unwrapped: Self::Unwrapped) -> Self;
    /// Run a closure for each value in this set, using the
    /// representative value.
    /// 
    /// # Safety
    /// 
    /// This is unsafe because it assumes that the caller is on the correct
    /// thread.
    unsafe fn for_each_representative<F>(&self, f: F)
    where
        F: FnMut(usize);

    #[doc(hidden)]
    fn __tag_marker(_t: Tag) {}
}

unsafe impl<T: Compatible, Tag> Wrapped<Tag> for Object<T, Tag> {
    type Unwrapped = T;

    unsafe fn unwrap(self) -> Self::Unwrapped {
        self.into_inner_unchecked()
    }

    unsafe fn wrap(unwrapped: Self::Unwrapped) -> Self {
        Self::new_unchecked(unwrapped)
    }

    unsafe fn for_each_representative<F>(&self, mut f: F)
    where
        F: FnMut(usize),
    {
        f(self.get_unchecked().representative());
    }
}

unsafe impl<'lt, T: Compatible, Tag> Wrapped<Tag> for &'lt [Object<T, Tag>] {
    type Unwrapped = &'lt [T];

    unsafe fn unwrap(self) -> Self::Unwrapped {
        // SAFETY: Object is transparent, so this cast is sound
        mem::transmute(self)
    }

    unsafe fn wrap(unwrapped: Self::Unwrapped) -> Self {
        // SAFETY: Object is transparent, so this cast is sound
        mem::transmute(unwrapped)
    }

    unsafe fn for_each_representative<F>(&self, mut f: F)
    where
        F: FnMut(usize),
    {
        for object in self.iter() {
            object.for_each_representative(&mut f);
        }
    }
}

macro_rules! wrap_tuple {
    () => {
        unsafe impl<Tag> Wrapped<Tag> for () {
            type Unwrapped = ();

            unsafe fn unwrap(self) -> Self::Unwrapped {}

            unsafe fn wrap(_unwrapped: Self::Unwrapped) -> Self {}

            unsafe fn for_each_representative<F>(&self, _f: F)
                where
                    F: FnMut(usize) {}
        }
    };
    ($current: ident $($gen: ident)*) => {
        // go down a level
        wrap_tuple!($($gen)*);

        #[allow(non_snake_case)]
        unsafe impl<Tag, $current: Wrapped<Tag>, $($gen: Wrapped<Tag>),*> Wrapped<Tag>
            for ($current, $($gen),*) {

            type Unwrapped = ($current::Unwrapped, $($gen::Unwrapped),*);

            unsafe fn unwrap(self) -> Self::Unwrapped {
                let ($current, $($gen),*) = self;
                ($current.unwrap(), $($gen.unwrap()),*)
            }

            unsafe fn wrap(unwrapped: Self::Unwrapped) -> Self {
                let ($current, $($gen),*) = unwrapped;
                (Wrapped::wrap($current), $(Wrapped::wrap($gen)),*)
            }

            unsafe fn for_each_representative<Func>(&self, mut f: Func)
                where
                    Func: FnMut(usize) {
                let (ref $current, $(ref $gen),*) = self;

                $current.for_each_representative(&mut f);
                $($gen.for_each_representative(&mut f);)*
            }
        }
    }
}

wrap_tuple! {
    A B C D E F G H I J
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
