use anyhow::{Result, bail};
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::Duration,
};
use tokio::sync::Notify;
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    Interactive,
    Background,
}
#[derive(Default)]
struct Group {
    active: bool,
    queue: VecDeque<(u64, Priority)>,
}
#[derive(Default)]
struct State {
    groups: HashMap<String, Group>,
    next: u64,
}
#[derive(Default, Clone)]
pub struct Scheduler {
    inner: Arc<Mutex<State>>,
    changed: Arc<Notify>,
}
pub struct Permit {
    scheduler: Scheduler,
    key: String,
}
struct Waiting {
    scheduler: Scheduler,
    key: String,
    id: u64,
}
impl Drop for Waiting {
    fn drop(&mut self) {
        if let Some(g) = self.scheduler.inner.lock().groups.get_mut(&self.key) {
            g.queue.retain(|(id, _)| *id != self.id);
        }
        self.scheduler.changed.notify_waiters();
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        if let Some(g) = self.scheduler.inner.lock().groups.get_mut(&self.key) {
            g.active = false;
        }
        self.scheduler.changed.notify_waiters();
    }
}
impl Scheduler {
    pub async fn acquire(
        &self,
        key: &str,
        priority: Priority,
        deadline: Duration,
    ) -> Result<Permit> {
        let id = {
            let mut s = self.inner.lock();
            s.next += 1;
            let id = s.next;
            let g = s.groups.entry(key.into()).or_default();
            if g.queue.len() >= 32 {
                bail!("Model host queue is full");
            }
            g.queue.push_back((id, priority));
            id
        };
        let waiting = Waiting {
            scheduler: self.clone(),
            key: key.into(),
            id,
        };
        let until = tokio::time::Instant::now() + deadline;
        loop {
            // Register before checking under the lock, preventing a lost wakeup.
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let mut s = self.inner.lock();
                let g = s.groups.get_mut(key).unwrap();
                let next = g
                    .queue
                    .iter()
                    .find(|(_, p)| *p == Priority::Interactive)
                    .or_else(|| g.queue.front())
                    .map(|v| v.0);
                if !g.active && next == Some(id) {
                    g.active = true;
                    g.queue.retain(|(n, _)| *n != id);
                    drop(s);
                    drop(waiting);
                    return Ok(Permit {
                        scheduler: self.clone(),
                        key: key.into(),
                    });
                }
            }
            if tokio::time::timeout_at(until, notified).await.is_err() {
                bail!("AI is busy; deadline expired");
            }
        }
    }
    pub fn status(&self) -> Value {
        let s = self.inner.lock();
        json!({"groups":s.groups.iter().map(|(k,g)|json!({"group":k,"active":u8::from(g.active),"queued":g.queue.len(),"interactive":g.queue.iter().filter(|(_,p)|*p==Priority::Interactive).count(),"capacity":1})).collect::<Vec<_>>()})
    }
}
