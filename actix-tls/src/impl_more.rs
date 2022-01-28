/// A helper to implement `Deref` for a type.
#[macro_export]
macro_rules! deref {
    ($ty:ident $(<$($generic:ident),*>)? => $field:tt: $target:ty) => {
        impl $(<$($generic),*>)? ::core::ops::Deref for $ty $(<$($generic),*>)? {
            type Target = $target;

            fn deref(&self) -> &Self::Target {
                &self.$field
            }
        }
    };
}

/// A helper to implement `DerefMut` for a type.
#[macro_export]
macro_rules! deref_mut {
    ($ty:ident $(<$($generic:ident),*>)? => $field:tt) => {
        impl $(<$($generic),*>)? ::core::ops::DerefMut for $ty $(<$($generic),*>)? {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.$field
            }
        }
    };
}

/// A helper to implement `From` for a unit struct.
#[macro_export]
macro_rules! from {
    ($from:ty => $ty:ident $(<$($generic:ident),*>)?) => {
        impl $(<$($generic),*>)? ::core::convert::From<$from> for $ty $(<$($generic),*>)? {
            fn from(from: $from) -> Self {
                Self(from)
            }
        }
    };
}

#[allow(unused_imports)]
pub(crate) use crate::{deref, deref_mut, from};
