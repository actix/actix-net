/// A helper to implement `Deref` for a type.
#[doc(hidden)]
#[macro_export]
macro_rules! deref {
    ($ty:ident $(<$($generic:ident),*>)? => $field:tt: $target:ty) => {
        impl $(<$($generic),*>)? ::std::ops::Deref for $ty $(<$($generic),*>)? {
            type Target = $target;

            fn deref(&self) -> &Self::Target {
                &self.$field
            }
        }
    };
}

/// A helper to implement `DerefMut` for a type.
#[doc(hidden)]
#[macro_export]
macro_rules! deref_mut {
    ($ty:ident $(<$($generic:ident),*>)? => $field:tt) => {
        impl $(<$($generic),*>)? ::std::ops::DerefMut for $ty $(<$($generic),*>)? {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.$field
            }
        }
    };
}

/// A helper to implement `From` for a unit struct.
#[doc(hidden)]
#[macro_export]
macro_rules! from {
    ($from:ty => $ty:ident $(<$($generic:ident),*>)?) => {
        impl $(<$($generic),*>)? ::std::convert::From<$from> for $ty $(<$($generic),*>)? {
            fn from(from: $from) -> Self {
                Self(from)
            }
        }
    };
}

/// A helper to implement `Display` and `Error` for an enum.
#[doc(hidden)]
#[macro_export]
macro_rules! enum_error {
    (match $ty:ident $(<$($generic:ident),*>)? |$f:ident| {$(
        $( #[source($source:expr)] )? $variant:pat => $fmt:expr $(,)?
    ),*}) => {
        #[allow(unused)]
        impl $(<$($generic),*>)? ::std::fmt::Display for $ty $(<$($generic),*>)? {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                use $ty::*;

                let $f = f;
                let res = match self {$(
                    $variant => $fmt,
                )*};

                write!($f, "{}", res)
            }
        }

        impl $(<$($generic: ::std::fmt::Debug),*>)? ::std::error::Error for $ty $(<$($generic),*>)? {
            fn source(&self) -> Option<&(dyn ::std::error::Error + 'static)> {
                use $ty::*;

                match self {
                    $($( $variant => { Some($source) } )?)*
                    _ => None
                }
            }
        }
    };
}

#[doc(inline)]
pub use crate::{deref, deref_mut, enum_error, from};
