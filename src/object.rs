// MIT/Apache2 License

use core::{any::type_name, fmt, marker::PhantomData, mem::MaybeUninit};

/// A thread-unsafe object that only has true meaning in the context of a
/// `BreadThread` runtime.
#[repr(transparent)]
pub struct Object<Ty, Tag> {
    // the tag
    tag: PhantomData<Tag>,
    // the real object
    //
    // this will never drop, since it's in the Compatible contract
    // to never drop. that being said, we like to think of this as
    // "just a bunch of bits", so it's best to wrap it in MaybeUninit
    // anyhow
    object: MaybeUninit<Ty>,
}

impl<Ty: Copy, Tag> Copy for Object<Ty, Tag> {}

impl<Ty: Copy, Tag> Clone for Object<Ty, Tag> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Ty, Tag> fmt::Debug for Object<Ty, Tag> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Object")
            .field("object", &type_name::<Ty>())
            .field("tag", &type_name::<Tag>())
            .finish()
    }
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
            object: MaybeUninit::new(object),
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
        self.object.assume_init()
    }

    /// Get a reference to the inner object.
    ///
    /// # Safety
    ///
    /// This is unsafe because it assumes that the caller is on the correct
    /// thread.
    pub unsafe fn get_unchecked(&self) -> &Ty {
        &*self.object.as_ptr()
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
