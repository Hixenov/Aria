//! A string that must never be printed: tokens, authorization codes, PKCE verifiers.

use std::fmt;

/// Holds a secret. `Debug` and `Display` print `***`; only [`Secret::expose`] gives the value.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: String) -> Secret {
        Secret(value)
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_print_as_stars() {
        let s = Secret::new("hunter2".to_owned());
        assert_eq!(format!("{:?} {}", s, s), "*** ***");
        assert_eq!(s.expose(), "hunter2");
    }
}
