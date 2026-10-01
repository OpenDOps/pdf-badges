use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// One registration row kept on the device.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registration {
    pub id: String,
    pub name: String,
}

/// Local copy of registration data.
///
/// `write_batch` holds the lock for that batch only. Lookups take the same
/// lock and release it before the handler awaits again.
#[derive(Default)]
pub struct Store {
    rows: Mutex<HashMap<String, Registration>>,
}

impl Store {
    pub fn new() -> Self {
        Self {
            rows: Mutex::new(HashMap::new()),
        }
    }

    pub fn write_batch(&self, rows: Vec<Registration>) {
        let mut guard = self.rows.lock().unwrap_or_else(|err| err.into_inner());
        for row in rows {
            guard.insert(row.id.clone(), row);
        }
    }

    pub fn lookup(&self, id: &str) -> Option<Registration> {
        let guard = self.rows.lock().unwrap_or_else(|err| err.into_inner());
        guard.get(id).cloned()
    }
}
