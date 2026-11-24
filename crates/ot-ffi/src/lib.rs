use std::future::Future;
use std::sync::LazyLock;

use tokio::runtime::Runtime;

mod client;
mod files;
mod local;
mod types;

pub use client::OtClient;
pub use files::{FileItem, FileSink, FileSource, TransferListener};
pub use local::{
    clear_history, device_state, forget_device, group_snapshot, history, invite_pending, join,
    join_invite, mark_opened, opened, set_file_uri, set_server_url,
};
pub use types::*;

uniffi::setup_scaffolding!();

static RUNTIME: LazyLock<Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("ot-core")
        .enable_all()
        .build()
        .expect("runtime tokio du cœur")
});

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

#[uniffi::export(with_foreign)]
pub trait SecretStore: Send + Sync {
    fn get(&self, name: String) -> Result<Option<Vec<u8>>, StoreError>;
    fn put(&self, name: String, value: Vec<u8>) -> Result<(), StoreError>;
    fn delete(&self, name: String) -> Result<(), StoreError>;
}

#[uniffi::export]
pub fn core_version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

#[uniffi::export]
pub fn classify_qr(text: String) -> QrKind {
    if let Ok(uri) = ot_core::PairUri::parse(&text) {
        return QrKind::Join { server_url: uri.server_url, code: uri.code };
    }
    match ot_core::InviteUri::parse(&text) {
        Ok(uri) => return QrKind::Invite { server_url: uri.server_url, code: uri.code },
        Err(e) if text.trim().starts_with(ot_core::INVITE_URI_PREFIX) => {
            return QrKind::Invalid { reason: e.to_string() };
        }
        Err(_) => {}
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
