//! Which secrets each consumer may see.

use std::collections::{BTreeMap, HashMap};

use crate::caller::CallerId;
use crate::secret::SecretValue;

/// The secrets each consumer may see, with their values.
///
/// The only lookup is by caller and name. A name another consumer holds and a name nobody
/// holds look the same to the caller: absent.
#[derive(Debug, Default, Clone)]
pub struct Policy {
    consumers: HashMap<CallerId, BTreeMap<String, SecretValue>>,
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

    /// The consumers this policy knows, in no particular order.
    pub fn consumers(&self) -> impl Iterator<Item = &CallerId> {
        self.consumers.keys()
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
}
