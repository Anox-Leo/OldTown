use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use ot_proto::DeviceId;
use tokio::sync::Notify;

#[derive(Debug)]
pub struct Session {
    pub id: u64,
    pub wake: Notify,
    pub kick: Notify,
    pub directory: Notify,
}

#[derive(Debug, Default)]
pub struct Registry {
    next_id: AtomicU64,
    sessions: Mutex<HashMap<DeviceId, Arc<Session>>>,
}

impl Registry {
    pub fn register(&self, device: &DeviceId) -> Arc<Session> {
        let session = Arc::new(Session {
            id: self.next_id.fetch_add(1, Ordering::Relaxed),
            wake: Notify::new(),
            kick: Notify::new(),
            directory: Notify::new(),
        });
        let old = self.lock().insert(device.clone(), session.clone());
        if let Some(old) = old {
            old.kick.notify_one();
        }
        session
    }

    pub fn unregister(&self, device: &DeviceId, session: &Session) {
        let mut sessions = self.lock();
        if sessions.get(device).is_some_and(|s| s.id == session.id) {
            sessions.remove(device);
        }
    }

    pub fn wake(&self, device: &DeviceId) {
        if let Some(s) = self.lock().get(device) {
            s.wake.notify_one();
        }
    }

    pub fn kick(&self, device: &DeviceId) {
        if let Some(s) = self.lock().remove(device) {
            s.kick.notify_one();
        }
    }

    pub fn directory_changed<'a>(&self, devices: impl IntoIterator<Item = &'a DeviceId>) {
        let sessions = self.lock();
        for d in devices {
            if let Some(s) = sessions.get(d) {
                s.directory.notify_one();
            }
        }
    }

    pub fn directory_changed_all(&self, except: &DeviceId) {
        for (d, s) in self.lock().iter() {
            if d != except {
                s.directory.notify_one();
            }
        }
    }

    pub fn is_online(&self, device: &DeviceId) -> bool {
        self.lock().contains_key(device)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<DeviceId, Arc<Session>>> {
        self.sessions.lock().unwrap_or_else(|e| e.into_inner())
    }
}
