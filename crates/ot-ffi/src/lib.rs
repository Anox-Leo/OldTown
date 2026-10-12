//! Pont UniFFI entre `ot-core` et l'appli Android (étape 3, T2).
//!
//! Kotlin ne voit que des types simples ; tout le protocole et la crypto
//! restent en Rust. L'appli fournit un [`SecretStore`] (Keystore Android) où
//! ce pont range identité, configuration, état du groupe, messages vus et
//! derniers éléments reçus. Un seul [`OtClient`] par processus tient la
//! connexion au relais (note 0009) : c'est le service de premier plan.

use std::future::Future;
use std::sync::LazyLock;

use tokio::runtime::Runtime;

mod client;
mod local;
mod types;

pub use client::OtClient;
pub use local::{
    clear_history, device_state, forget_device, group_snapshot, history, join, set_server_url,
};
pub use types::*;

uniffi::setup_scaffolding!();

/// Runtime tokio du cœur, partagé par tout le processus. Les fonctions
/// `async` exportées y lancent leur travail : l'exécuteur de Kotlin ne fait
/// qu'attendre le résultat.
static RUNTIME: LazyLock<Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("ot-core")
        .enable_all()
        .build()
        // Invariant : seul un système incapable de créer deux threads échoue
        // ici, et rien ne peut fonctionner sans.
        .expect("runtime tokio du cœur")
});

/// Lance `fut` sur le runtime du cœur et attend son résultat.
async fn on_runtime<T, F>(fut: F) -> Result<T, OtError>
where
    T: Send + 'static,
    F: Future<Output = Result<T, OtError>> + Send + 'static,
{
    RUNTIME
        .spawn(fut)
        .await
        .map_err(|e| OtError::Internal { message: format!("tâche interrompue : {e}") })?
}

/// Stockage fourni par l'appli : octets nommés, chiffrés au repos (Keystore
/// sur Android). Les noms sont courts, en ASCII (`device.key`,
/// `group.json`…). Appelé depuis plusieurs threads.
#[uniffi::export(with_foreign)]
pub trait SecretStore: Send + Sync {
    /// Valeur de `name`, `None` si elle n'existe pas.
    fn get(&self, name: String) -> Result<Option<Vec<u8>>, StoreError>;
    /// Remplace la valeur de `name` (écriture atomique).
    fn put(&self, name: String, value: Vec<u8>) -> Result<(), StoreError>;
    /// Efface `name` ; sans effet s'il n'existe pas.
    fn delete(&self, name: String) -> Result<(), StoreError>;
}

/// Version du cœur Rust, affichée dans les réglages.
#[uniffi::export]
pub fn core_version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

/// Nature d'un QR scanné, pour choisir l'écran suivant.
#[uniffi::export]
pub fn classify_qr(text: String) -> QrKind {
    if let Ok(uri) = ot_core::PairUri::parse(&text) {
        return QrKind::Join { server_url: uri.server_url, code: uri.code };
    }
    match ot_core::PairingOffer::parse(&text) {
        Ok(offer) => QrKind::Sponsor { name: offer.name, device_id: offer.device_id.to_string() },
        Err(e) if text.trim().starts_with(ot_core::OFFER_PREFIX) => {
            QrKind::Invalid { reason: e.to_string() }
        }
        Err(_) if text.trim().starts_with(ot_core::PAIR_URI_PREFIX) => {
            QrKind::Invalid { reason: "QR « rejoindre » invalide".into() }
        }
        Err(_) => QrKind::Unknown,
    }
}

/// Modules d'un QR code pour `text`, à dessiner par l'appli.
#[uniffi::export]
pub fn qr_code(text: String) -> Result<QrCode, OtError> {
    let code = qrcode::QrCode::new(text.as_bytes())
        .map_err(|e| OtError::Invalid { message: format!("QR impossible : {e}") })?;
    let size = code.width() as u32;
    let modules = code.to_colors().into_iter().map(|c| c == qrcode::Color::Dark).collect();
    Ok(QrCode { size, modules })
}

#[cfg(test)]
mod tests;
