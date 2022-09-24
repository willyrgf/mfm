// Copyright 2020. The Tari Project
//
// Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
// following conditions are met:
//
// 1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
// disclaimer.
//
// 2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
// following disclaimer in the documentation and/or other materials provided with the distribution.
//
// 3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
// products derived from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
// INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
// SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
// SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
// WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
// USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

//! A wrapper to conceal secrets when output into logs or displayed.

use std::{fmt, ops::DerefMut};

use serde::{Deserialize, Serialize};

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
