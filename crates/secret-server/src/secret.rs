//! The secret value type.

use std::fmt;

use zeroize::Zeroizing;

/// The bytes of one secret, as read from the process environment at startup.
///
/// The value never leaves through formatting: `Debug` renders `<redacted>`, and there is no
/// `Display` or `Serialize`. The bytes are wiped on drop.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretValue(Zeroizing<Vec<u8>>);

impl SecretValue {
    /// Wraps the bytes of a secret.
    pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
        Self(Zeroizing::new(bytes.into()))
    }

    /// The raw bytes, for building the SDS resource. Crate-private so that no caller of the
    /// library can print or log them by accident.
    pub(crate) fn expose(&self) -> &[u8] {
        &self.0
    }

    /// Whether the value is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretValue(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_the_value() {
        let value = SecretValue::new("hunter2-very-secret");
        let rendered = format!("{value:?} {:#?}", Some(&value));
        assert!(!rendered.contains("hunter2"), "{rendered}");
        assert!(rendered.contains("<redacted>"));
    }
}
