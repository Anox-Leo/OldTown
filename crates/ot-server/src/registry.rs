//! Registre des sessions actives : une seule par appareil, la plus récente
//! remplace l'ancienne.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use ot_proto::DeviceId;
use tokio::sync::Notify;

#[derive(Debug)]
pub struct Session {
    pub id: u64,
    /// Réveille la session : de nouveaux messages l'attendent.
    pub wake: Notify,
    /// Demande à la session de se fermer (remplacée par une plus récente).
    pub kick: Notify,
}

#[derive(Debug, Default)]
pub struct Registry {
    next_id: AtomicU64,
    sessions: Mutex<HashMap<DeviceId, Arc<Session>>>,
}

impl Registry {
    /// Enregistre une nouvelle session et ferme la précédente du même appareil.
    pub fn register(&self, device: &DeviceId) -> Arc<Session> {
        let session = Arc::new(Session {
            id: self.next_id.fetch_add(1, Ordering::Relaxed),
            wake: Notify::new(),
            kick: Notify::new(),
        });
        let old = self.lock().insert(device.clone(), session.clone());
        if let Some(old) = old {
            old.kick.notify_one();
        }
        session
    }

    /// Retire la session si c'est toujours celle enregistrée pour l'appareil.
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

    pub fn is_online(&self, device: &DeviceId) -> bool {
        self.lock().contains_key(device)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<DeviceId, Arc<Session>>> {
        self.sessions.lock().unwrap_or_else(|e| e.into_inner())
    }
}
