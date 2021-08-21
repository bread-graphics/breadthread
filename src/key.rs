// MIT/Apache2 License

use crate::{mutex::RwLock, AddOrRemovePtr};
use std::{collections::HashMap, marker::PhantomData, num::NonZeroUsize, ptr::NonNull};

/// The type that a `Key` can have.
pub trait KeyType {
    /// The type that this key represents a pointer to.
    type Real;
    /// A unique identifier that identifies the key's type.
    const IDENTIFIER: usize;
}

/// A key. This is a wrapper around a non-zero `usize` that usually represents a pointer. This object represents
/// a foreign object, usually managed by the API that the bread thread is managing. This is provided as a utility
/// so you know that the pointer you have is a pointer to the object you're talking about. The thread itself is
/// able to verify that the pointer not only is not dangling, but refers to the type you are talking about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Key<Type> {
    key: NonZeroUsize,
    _marker: PhantomData<Type>,
}

impl<Type> Key<Type> {
    /// Create a new `Key` from a `NonZeroUsize`.
    #[inline]
    pub fn from_raw(nzu: NonZeroUsize) -> Key<Type> {
        Key {
            key: nzu,
            _marker: PhantomData,
        }
    }

    /// Get the `NonZeroUsize` from this `Key`.
    #[inline]
    pub fn into_raw(self) -> NonZeroUsize {
        self.key
    }

    /// Cast this `Key` into a `Key` of another type.
    #[inline]
    pub fn cast<OtherType>(self) -> Key<OtherType> {
        Key {
            key: self.key,
            _marker: PhantomData,
        }
    }
}

impl<Type: KeyType> Key<Type> {
    /// Create a new `Key` from a non-null pointer.
    #[inline]
    pub fn from_non_null(nn: NonNull<Type::Real>) -> Key<Type> {
        Key::from_raw(
            NonZeroUsize::new(nn.as_ptr() as *mut () as usize)
                .expect("NonNull is expected to be null, this should always work"),
        )
    }

    /// Create a new `Key` from a pointer. This function returns `None` if the pointer is null.
    #[inline]
    pub fn from_ptr(ptr: *mut Type::Real) -> Option<Key<Type>> {
        NonZeroUsize::new(ptr as *mut () as usize).map(Key::from_raw)
    }

    /// Convert this `Key` into a non-null pointer.
    #[inline]
    pub fn as_non_null(self) -> NonNull<Type::Real> {
        NonNull::new(self.key.get() as *mut () as *mut Type::Real)
            .expect("Should always be non-null")
    }

    /// Convert this `Key` into a pointer.
    #[inline]
    pub fn as_ptr(self) -> *mut Type::Real {
        self.key.get() as *mut Type::Real
    }

    /// Get the type identifier for this key.
    #[inline]
    pub fn identifier(self) -> usize {
        Type::IDENTIFIER
    }

    /// Get a bundle between the identifier and the actual pointer.
    #[inline]
    pub fn verifiable(self) -> (NonZeroUsize, usize) {
        (self.key, Type::IDENTIFIER)
    }
}

/// A server that stores identification for `Key`s.
#[derive(Debug)]
pub(crate) struct KeyServer {
    // this *could* be a dashmap; i really dont mind the extra dependency. however, i think creating/destroying
    // pointers will not be a hot operation, so this might be faster and use less memory in the end
    pointers: RwLock<HashMap<NonZeroUsize, usize>>,
}

impl KeyServer {
    #[inline]
    pub(crate) fn new() -> KeyServer {
        KeyServer {
            pointers: RwLock::new(HashMap::new()),
        }
    }

    #[inline]
    pub(crate) fn process_new_pointers<I: IntoIterator<Item = AddOrRemovePtr>>(&self, pointers: I) {
        let mut lock = None;
        pointers.into_iter().for_each(|dir| match dir {
            AddOrRemovePtr::DoNothing => {}
            AddOrRemovePtr::AddPtr(ptr, ty) => {
                lock.get_or_insert_with(|| self.pointers.write())
                    .insert(ptr, ty);
            }
            AddOrRemovePtr::RemovePtr(ptr) => {
                lock.get_or_insert_with(|| self.pointers.write())
                    .remove(&ptr);
            }
        });
    }

    #[inline]
    pub(crate) fn verify_pointers<I: IntoIterator<Item = (NonZeroUsize, usize)>>(
        &self,
        pointers: I,
    ) -> Result<(), NonZeroUsize> {
        let lock = self.pointers.read();
        pointers.into_iter().try_for_each(|(pointer, ty)| {
            if lock.get(&pointer).copied() == Some(ty) {
                Ok(())
            } else {
                Err(pointer)
            }
        })
    }
}

/// Create a type that internally wraps around `Key`.
///
/// # Example
///
/// ```
/// use breadthread::Key;
/// use std::num::NonZeroUsize;
///
/// pub struct ForeignPointerType { no_data: [u8; 0] }
///
/// breadthread::key_type! {
///     /// Foobar.
///     pub struct MyType(ForeignPointerType) : [ForeignType, 0x1337];
/// }
///
/// let key = MyType::from_raw(NonZeroUsize::new(0x12).unwrap());
/// assert_eq!(key.into_raw(), NonZeroUsize::new(0x12).unwrap());
/// let _inner_key: Key<ForeignType> = key.into_key();
/// assert_eq!(key.identifier(), 0x1337);
/// ```
///
/// # Note
///
/// The new type already derives `Debug`, `Copy`, `Clone`, `PartialOrd`, `Ord`, `PartialEq`, `Eq`,
/// and `Hash`. Deriving these yourself will likely result in an error.
#[macro_export]
macro_rules! key_type {
    (
        $(#[$meta: meta])*
        $vis: vis struct $tyname: ident ( $foreign: ident ) : [$traitname: ident, $value: expr] ;
    ) => {
        #[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
        #[doc = "An object used to parameterize `Key`s representing a foreign pointer to"]
        #[doc = concat!("an object of type `", stringify!($foreign), "`.")]
        $vis struct $traitname;

        impl $crate::KeyType for $traitname {
            type Real = $foreign;
            const IDENTIFIER: usize = $value;
        }

        $(#[$meta])*
        #[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        $vis struct $tyname ( $crate::Key<$traitname> );

        impl $tyname {
            #[doc = concat!("Creates a new `", stringify!($tyname), "` from an appropriately typed key.")]
            #[inline]
            pub fn from_key(key: $crate::Key<$traitname>) -> Self {
                Self(key)
            }

            #[doc = concat!("Gets the inner key for a `", stringify!($tyname), "`.")]
            #[inline]
            pub fn into_key(self) -> $crate::Key<$traitname> {
                self.0
            }

            #[doc = concat!("Constructs a `", stringify!($tyname), "` from a non-zero `usize` representing a")]
            #[doc = concat!("pointer to a `", stringify!($foreign), "`.")]
            #[inline]
            pub fn from_raw(raw: std::num::NonZeroUsize) -> Self {
                Self::from_key($crate::Key::from_raw(raw))
            }

            #[doc = concat!("Get the backing `NonZeroUsize` from this `", stringify!($tyname), "`.")]
            #[inline]
            pub fn into_raw(self) -> NonZeroUsize {
                self.into_key().into_raw()
            }

            #[doc = concat!("Creates a new `", stringify!($tyname), "` from a non-null pointer to a ")]
            #[doc = concat!("`", stringify!($foreign), "`.")]
            #[inline]
            pub fn from_non_null(nn: std::ptr::NonNull<$foreign>) -> Self {
                Self::from_key($crate::Key::from_non_null(nn))
            }

            #[doc = concat!("Creates a new `", stringify!($tyname), "` from a pointer to a ")]
            #[doc = concat!("`", stringify!($foreign), "`. If the pointer is null, this function returns")]
            #[doc = "`None`."]
            #[inline]
            pub fn from_ptr(ptr: *mut $foreign) -> Option<Self> {
                $crate::Key::from_ptr(ptr).map(Self::from_key)
            }

            #[doc = concat!("Converts this `", stringify!($tyname), "` into a non-null pointer to a ")]
            #[doc = concat!("`", stringify!($foreign), "`.")]
            #[inline]
            pub fn as_non_null(self) -> std::ptr::NonNull<$foreign> {
                self.0.as_non_null()
            }

            #[doc = concat!("Converts this `", stringify!($tyname), "` into a pointer to a `")]
            #[doc = concat!(stringify!($foreign), "`.")]
            #[inline]
            pub fn as_ptr(self) -> *mut $foreign {
                self.0.as_ptr()
            }

            #[doc = concat!("Get an identifier that identifies that this `", stringify!($tyname), "` ")]
            #[doc = concat!("refers to an object of type `", stringify!($foreign), "`.")]
            #[inline]
            pub fn identifier(self) -> usize {
                $value
            }

            #[doc = concat!("Get a bundle consisting of this `", stringify!($tyname), "`'s backing pointer")]
            #[doc = "and its identifier. Useful for passing into `Directive::pointers()`"]
            #[inline]
            pub fn verifiable(self) -> (NonZeroUsize, usize) {
                (self.into_raw(), $value)
            }
        }
    }
}
