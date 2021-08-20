// MIT/Apache2 License

use crate::{mutex::RwLock, AddOrRemovePtr};
use std::{collections::HashMap, marker::PhantomData, num::NonZeroUsize, ptr::NonNull};

/// The type that a `Key` can have.
pub trait KeyType {
    /// The type that this key represents a pointer to.
    type Real: ?Sized;
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
}

impl<Type: KeyType> Key<Type> {
    /// Create a new `Key` from a non-null pointer.
    #[inline]
    pub fn from_non_null(nn: NonNull<Type::Real>) -> Key<Type> {
        Key::from_raw(
            NonZeroUsize::new(nn.as_ptr() as usize)
                .expect("NonNull is expected to be null, this should always work"),
        )
    }

    /// Create a new `Key` from a pointer. This function returns `None` if the pointer is null.
    #[inline]
    pub fn from_ptr(ptr: *mut Type::Real) -> Option<Key<Type>> {
        Some(Key::from_raw(NonZeroUsize::new(ptr as usize)?))
    }

    /// Convert this `Key` into a non-null pointer.
    #[inline]
    pub fn as_non_null(self) -> NonNull<Type::Real> {
        NonNull::new(self.key.get() as *mut Type::Real).expect("Should always be non-null")
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
#[derive(Debug, Clone)]
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
        pointers.into_iter().fold(None, |mut lock, dir| {
            match dir {
                AddOrRemovePtr::DoNothing => {}
                AddOrRemovePtr::AddPtr(ptr, ty) => match lock {
                    Some(lock) => {
                        lock.insert(ptr, ty);
                    }
                    lock => {
                        lock.insert(self.pointers.write()).insert(ptr, ty);
                    }
                },
                AddOrRemovePtr::RemovePtr(ptr) => match lock {
                    Some(lock) => {
                        lock.remove(&ptr);
                    }
                    lock => {
                        lock.insert(self.pointers.write()).remove(&ptr);
                    }
                },
            }

            lock
        });
    }

    #[inline]
    pub(crate) fn verify_pointers<I: IntoIterator<Item = (NonZeroUsize, usize)>>(
        &self,
        pointers: I,
    ) -> bool {
        let lock = self.pointers.read();
        pointers
            .into_iter()
            .all(|(pointer, ty)| lock.get(&pointer) == Some(ty))
    }
}
