// MIT/Apache2 License

use crate::{Driver, Pinned, Value, Wrapped};
use alloc::{boxed::Box, vec::Vec};

/// A directive to run a thread-unsafe operation using thread-unsafe primitives.
pub(crate) struct Directive<'lt> {
    // this is actually a wrapper around a closure that does everything we
    // need to for now
    closure: Box<dyn FnOnce(&mut Driver<'lt, Pinned>) + Send + 'lt>,
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
        let (ret_slot, in_slot) = Value::new();

        let closure = Box::new(move |driver: &mut Driver<'lt, Pinned>| {
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
                thread_safe_value,
                thread_unsafe_value,
                deleted_values,
            } = op(unwrapped);

            // wrap the out
            let ts_out = unsafe { TsOutput::wrap(thread_unsafe_value) };

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
            in_slot.put((thread_safe_value, ts_out));
        });

        (Self { closure }, ret_slot)
    }

    pub(crate) fn run(self, driver: &mut Driver<'lt, Pinned>) {
        (self.closure)(driver);
    }
}

/// The expected output of a directive.
pub struct DirectiveOutput<NtsOutput, TsOutput> {
    /// The output value that is thread safe
    pub thread_safe_value: NtsOutput,
    /// The output value that is thread unsafe
    pub thread_unsafe_value: TsOutput,
    /// The list of values that can be removed from the driver's known values
    pub deleted_values: Vec<usize>,
}
