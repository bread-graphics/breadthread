// MIT/Apache2 License

#![cfg(feature = "std")]

use crate::Wrapped;
use core::{any::TypeId, cell::Cell};

std::thread_local! {
    static CURRENT_THREAD_ID: Cell<Option<TypeId>> = Cell::new(None);
}

/// Indicate that a driver has been pinned to the current thread.
pub(crate) fn set_thread_id(tid: TypeId) {
    CURRENT_THREAD_ID
        .try_with(|slot| {
            slot.set(Some(tid));
        })
        .ok();
}

/// Temporarily unwrap a set of values for usage if we're on the right thread.
pub fn if_tagged_thread<Tag: 'static, Input: Wrapped<Tag>, Output: Wrapped<Tag>>(
    input: Input,
    f: impl FnOnce(Input::Unwrapped) -> Output::Unwrapped,
) -> Option<Output> {
    // see if we have the right id
    let tid = TypeId::of::<Tag>();

    if let Some(current_tid) = CURRENT_THREAD_ID.try_with(|tid| tid.get()).ok().flatten() {
        if current_tid == tid {
            // we're on the thread, we can unwrap and run
            // TODO: if the thread is in fallback mode, we can't actually do this?
            let input = unsafe { input.unwrap() };
            let output = f(input);
            return Some(unsafe { Output::wrap(output) });
        }
    }

    None
}
