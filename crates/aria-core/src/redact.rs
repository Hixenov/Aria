//! Wrapper that never prints its inner value.

use std::fmt;

/// Holds a sensitive value; `Debug` and `Display` print `***`.
#[derive(Clone)]
pub struct Redacted<T>(T);

impl<T> Redacted<T> {
    /// Wraps a value.
    pub fn new(value: T) -> Self {
        Self(value)
    }

    /// Returns a reference to the wrapped value.
    pub fn expose(&self) -> &T {
        &self.0
    }
}

impl<T> fmt::Debug for Redacted<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

impl<T> fmt::Display for Redacted<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacted_never_prints_inner() {
        let r = Redacted::new("secret-token".to_string());
        assert_eq!(format!("{r:?}"), "***");
        assert_eq!(format!("{r}"), "***");
        assert_eq!(r.expose(), "secret-token");
    }
}
