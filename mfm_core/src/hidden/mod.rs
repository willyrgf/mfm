/// ! A wrapper to conceal secrets when output into logs or displayed.
//
use serde::{Deserialize, Serialize};
use std::{fmt, ops::DerefMut};

/// A simple struct with a single inner value to wrap content of any type.
#[derive(Copy, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Hidden<T> {
    inner: T,
}

impl<T> Hidden<T> {
    /// Returns ownership of the inner value discarding the wrapper.
    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<T> From<T> for Hidden<T> {
    fn from(inner: T) -> Self {
        Hidden { inner }
    }
}

/// Defines a masked value for the type to output as debug information. Concealing the secrets within.
impl<T> fmt::Debug for Hidden<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hidden<{}>", std::any::type_name::<T>())
    }
}

/// Defines a masked value for the type to display. Concealing the secrets within.
impl<T> fmt::Display for Hidden<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hidden<{}>", std::any::type_name::<T>())
    }
}

/// Attempts to make the wrapper more transparent by having deref return a reference to the inner value.
impl<T> std::ops::Deref for Hidden<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T> DerefMut for Hidden<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl<T: PartialEq> PartialEq for Hidden<T> {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl<T: Eq> Eq for Hidden<T> {}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn into_applies_wrapper_deref_removes_it() {
        let wrapped: Hidden<u8> = 42.into();
        assert_eq!(42, *wrapped)
    }
}
