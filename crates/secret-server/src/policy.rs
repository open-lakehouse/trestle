//! Which secrets each consumer may see, and which keys it may use.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::{Arc, RwLock};

use tokio::sync::watch;

use crate::caller::CallerId;
use crate::keyring::KeyHandle;
use crate::secret::SecretValue;

/// The secrets each consumer may see, with their values, and the keys it may wrap and unwrap
/// under, with their keyrings.
///
/// The only lookups are by caller and name. A name another consumer holds and a name nobody
/// holds look the same to the caller: absent. Clones share the values and the keyrings, so a
/// [`SecretCell::set`] or a [`KeyHandle::replace`] reaches every clone, and
/// [`changes`](Self::changes) tells open streams that a value changed.
#[derive(Debug, Clone)]
pub struct Policy {
    consumers: HashMap<CallerId, BTreeMap<String, SecretCell>>,
    keys: HashMap<CallerId, BTreeMap<String, KeyHandle>>,
    changes: Arc<watch::Sender<u64>>,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            consumers: HashMap::new(),
            keys: HashMap::new(),
            changes: Arc::new(watch::channel(0).0),
        }
    }
}

impl Policy {
    /// An empty policy: nobody sees anything.
    pub fn new() -> Self {
        Self::default()
    }

    /// A cell holding `value` whose changes this policy's [`changes`](Self::changes) reports.
    /// Allow it for every consumer that shares the value ([`allow_cell`](Self::allow_cell)).
    pub fn cell(&self, value: SecretValue) -> SecretCell {
        SecretCell {
            inner: Arc::new(CellInner {
                value: RwLock::new((value, 0)),
                changes: self.changes.clone(),
            }),
        }
    }

    /// Lets `caller` see `name` with `value`, replacing an earlier value for the same name.
    pub fn allow(&mut self, caller: CallerId, name: impl Into<String>, value: SecretValue) {
        let cell = self.cell(value);
        self.allow_cell(caller, name, cell);
    }

    /// Lets `caller` see `name` with the value in `cell`, which may change later.
    pub fn allow_cell(&mut self, caller: CallerId, name: impl Into<String>, cell: SecretCell) {
        self.consumers
            .entry(caller)
            .or_default()
            .insert(name.into(), cell);
    }

    /// The current value of `name`, if `caller` may see it.
    pub fn get(&self, caller: &CallerId, name: &str) -> Option<SecretValue> {
        self.cell_of(caller, name).map(SecretCell::value)
    }

    /// The cell of `name`, if `caller` may see it.
    pub fn cell_of(&self, caller: &CallerId, name: &str) -> Option<&SecretCell> {
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

    /// Follows changes to any secret value: the counter goes up each time one changes.
    pub fn changes(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }
}

/// One secret's value, which a live source may replace. Clones share it.
///
/// Each change bumps the cell's generation and the policy's change counter, so a stream that
/// sent the value can tell that it is stale.
#[derive(Clone)]
pub struct SecretCell {
    inner: Arc<CellInner>,
}

struct CellInner {
    value: RwLock<(SecretValue, u64)>,
    changes: Arc<watch::Sender<u64>>,
}

impl SecretCell {
    /// The value as it is now.
    pub fn value(&self) -> SecretValue {
        self.read().0.clone()
    }

    /// How many times the value has changed.
    pub fn generation(&self) -> u64 {
        self.read().1
    }

    /// Replaces the value. Returns `false`, and changes nothing, when it is the same value.
    pub fn set(&self, value: SecretValue) -> bool {
        {
            let mut current = self
                .inner
                .value
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if current.0 == value {
                return false;
            }
            *current = (value, current.1 + 1);
        }
        self.inner.changes.send_modify(|n| *n += 1);
        true
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, (SecretValue, u64)> {
        self.inner
            .value
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl fmt::Debug for SecretCell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecretCell")
            .field("value", &"<redacted>")
            .field("generation", &self.generation())
            .finish()
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

    #[test]
    fn a_shared_cell_changes_for_every_consumer() {
        let (a, b) = (CallerId::new("a"), CallerId::new("b"));
        let mut policy = Policy::new();
        let cell = policy.cell(SecretValue::new("v1"));
        policy.allow_cell(a.clone(), "planner", cell.clone());
        policy.allow_cell(b.clone(), "shared", cell.clone());
        let mut changes = policy.changes();
        assert!(
            !cell.set(SecretValue::new("v1")),
            "the same value is no change"
        );
        assert!(!changes.has_changed().unwrap());
        assert!(cell.set(SecretValue::new("v2")));
        assert!(changes.has_changed().unwrap());
        changes.borrow_and_update();
        assert_eq!(policy.get(&a, "planner"), Some(SecretValue::new("v2")));
        assert_eq!(policy.get(&b, "shared"), Some(SecretValue::new("v2")));
        assert_eq!(policy.cell_of(&a, "planner").unwrap().generation(), 1);
        let rendered = format!("{cell:?} {policy:?}");
        assert!(!rendered.contains("v2"), "{rendered}");
    }
}
