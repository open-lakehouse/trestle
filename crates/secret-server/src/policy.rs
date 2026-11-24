//! Which secrets each consumer may see, and which keys it may use.

use std::collections::{BTreeMap, HashMap};

use crate::caller::CallerId;
use crate::keyring::KeyHandle;
use crate::secret::SecretValue;

/// The secrets each consumer may see, with their values, and the keys it may wrap and unwrap
/// under, with their keyrings.
///
/// The only lookups are by caller and name. A name another consumer holds and a name nobody
/// holds look the same to the caller: absent. Clones share the keyrings, so a
/// [`KeyHandle::replace`] reaches every clone.
#[derive(Debug, Default, Clone)]
pub struct Policy {
    consumers: HashMap<CallerId, BTreeMap<String, SecretValue>>,
    keys: HashMap<CallerId, BTreeMap<String, KeyHandle>>,
}

impl Policy {
    /// An empty policy: nobody sees anything.
    pub fn new() -> Self {
        Self::default()
    }

    /// Lets `caller` see `name` with `value`, replacing an earlier value for the same name.
    pub fn allow(&mut self, caller: CallerId, name: impl Into<String>, value: SecretValue) {
        self.consumers
            .entry(caller)
            .or_default()
            .insert(name.into(), value);
    }

    /// The value of `name`, if `caller` may see it.
    pub fn get(&self, caller: &CallerId, name: &str) -> Option<&SecretValue> {
        self.consumers.get(caller)?.get(name)
    }

    /// Lets `caller` use the key `name` with `keyring`, replacing an earlier one.
    pub fn allow_key(&mut self, caller: CallerId, name: impl Into<String>, keyring: KeyHandle) {
        self.keys
            .entry(caller)
            .or_default()
            .insert(name.into(), keyring);
    }

    /// The keyring of `name`, if `caller` may use it.
    pub fn key(&self, caller: &CallerId, name: &str) -> Option<&KeyHandle> {
        self.keys.get(caller)?.get(name)
    }

    /// The consumers this policy knows, with secrets or keys, in no particular order.
    pub fn consumers(&self) -> impl Iterator<Item = &CallerId> {
        let mut all: Vec<&CallerId> = self.consumers.keys().chain(self.keys.keys()).collect();
        all.sort();
        all.dedup();
        all.into_iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_consumer_sees_only_its_own_names() {
        let a = CallerId::new("a");
        let b = CallerId::new("b");
        let mut policy = Policy::new();
        policy.allow(a.clone(), "planner", SecretValue::new("v"));
        assert!(policy.get(&a, "planner").is_some());
        assert!(policy.get(&b, "planner").is_none());
        assert!(policy.get(&a, "unknown").is_none());
        assert!(policy.get(&CallerId::new("nobody"), "planner").is_none());
    }

    #[test]
    fn a_consumer_uses_only_its_own_keys() {
        use base64::Engine;
        let json = format!(
            r#"{{"versions": {{"1": "{}"}}, "active": 1}}"#,
            base64::engine::general_purpose::STANDARD.encode([1u8; 32])
        );
        let keyring = crate::keyring::Keyring::from_json("kek", json.as_bytes()).unwrap();
        let a = CallerId::new("a");
        let mut policy = Policy::new();
        policy.allow_key(a.clone(), "kek", KeyHandle::new(keyring));
        assert!(policy.key(&a, "kek").is_some());
        assert!(policy.key(&CallerId::new("b"), "kek").is_none());
        assert!(policy.get(&a, "kek").is_none());
        assert_eq!(policy.consumers().count(), 1);
    }
}
