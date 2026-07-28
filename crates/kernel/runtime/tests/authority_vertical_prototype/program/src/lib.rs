#![forbid(unsafe_code)]
//! Value-only program-facing views for the authority-flow prototype.

/// A borrowed, value-only view of a capability request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestView<'a> {
    schema: &'a str,
    value: &'a [u8],
}

impl<'a> RequestView<'a> {
    /// Borrows a request value without journal or authorization metadata.
    pub const fn new(schema: &'a str, value: &'a [u8]) -> Self {
        Self { schema, value }
    }

    /// Returns the request schema.
    pub const fn schema(self) -> &'a str {
        self.schema
    }

    /// Returns the canonical request value bytes.
    pub const fn value(self) -> &'a [u8] {
        self.value
    }
}

/// A borrowed, value-only view of a capability observation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObservationView<'a> {
    schema: &'a str,
    value: &'a [u8],
}

impl<'a> ObservationView<'a> {
    /// Borrows an observation value without journal or authorization metadata.
    pub const fn new(schema: &'a str, value: &'a [u8]) -> Self {
        Self { schema, value }
    }

    /// Returns the observation schema.
    pub const fn schema(self) -> &'a str {
        self.schema
    }

    /// Returns the canonical observation value bytes.
    pub const fn value(self) -> &'a [u8] {
        self.value
    }
}
