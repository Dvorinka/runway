//! In-process event bus — replaces the Redis Streams update channel from
//! devpush. Single-binary model means pub/sub is a broadcast channel, not
//! an external service.
//!
//! Scopes:
//!   `project:{id}:updates`      — deployment status / alias changes (SSE)
//!   `deployment:{id}:logs`      — log lines for the log stream endpoint

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use tokio::sync::broadcast;

const CAPACITY: usize = 512;

#[derive(Clone, Default)]
pub struct EventBus {
    inner: Arc<RwLock<HashMap<String, broadcast::Sender<String>>>>,
}

impl EventBus {
    pub fn new() -> Self {
        Self::default()
    }

    fn sender(&self, scope: &str) -> broadcast::Sender<String> {
        let mut map = self.inner.write().unwrap();
        map.entry(scope.to_string())
            .or_insert_with(|| broadcast::channel(CAPACITY).0)
            .clone()
    }

    pub fn subscribe(&self, scope: &str) -> broadcast::Receiver<String> {
        self.sender(scope).subscribe()
    }

    /// Publish a JSON payload to a scope. No-op when nobody listens.
    pub fn publish(&self, scope: &str, payload: &serde_json::Value) {
        let map = self.inner.read().unwrap();
        if let Some(tx) = map.get(scope) {
            let _ = tx.send(payload.to_string());
        }
    }

    pub fn publish_line(&self, scope: &str, line: &str) {
        let map = self.inner.read().unwrap();
        if let Some(tx) = map.get(scope) {
            let _ = tx.send(line.to_string());
        }
    }

    /// Reap senders with no receivers so the map doesn't grow forever.
    pub fn gc(&self) {
        let mut map = self.inner.write().unwrap();
        map.retain(|_, tx| tx.receiver_count() > 0);
    }
}
