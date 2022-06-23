// MIT/Apache2 License

use crate::{Compatible, Object};
use core::mem;

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
