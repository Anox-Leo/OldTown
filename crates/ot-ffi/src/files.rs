//! Fichiers (étape 5, note 0026) : Kotlin fournit la lecture et l'écriture
//! (un `ContentResolver` : le Rust ne sait pas ouvrir une URI `content://`),
//! le cœur fait le reste. Aucune clé de fichier ne traverse le pont.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::StoreError;

/// Fichier à envoyer, lu par Kotlin. Appelé hors du thread principal.
#[uniffi::export(with_foreign)]
pub trait FileSource: Send + Sync {
    /// Taille du fichier, en octets.
    fn size(&self) -> u64;
    /// `len` octets à partir de `offset` (moins seulement en fin de fichier).
    fn read_at(&self, offset: u64, len: u32) -> Result<Vec<u8>, StoreError>;
}

/// Fichier reçu, écrit par Kotlin (MediaStore, `IS_PENDING` jusqu'à
/// `finish`). Les morceaux arrivent dans l'ordre.
#[uniffi::export(with_foreign)]
pub trait FileSink: Send + Sync {
    fn write_at(&self, offset: u64, data: Vec<u8>) -> Result<(), StoreError>;
    /// SHA-256 vérifié : rendre le fichier visible. Renvoie son URI.
    fn finish(&self) -> Result<String, StoreError>;
    /// Transfert abandonné : effacer ce qui a été écrit.
    fn abort(&self);
}

/// Progression d'un transfert, en octets.
#[uniffi::export(with_foreign)]
pub trait TransferListener: Send + Sync {
    fn on_progress(&self, done: u64, total: u64);
}

/// Fichier annoncé, tel qu'il est rangé dans l'historique.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(deny_unknown_fields)]
pub struct FileItem {
    pub blob_id: String,
    /// Nom nettoyé, sûr à écrire.
    pub name: String,
    pub mime: String,
    pub size: u64,
    /// URI du fichier enregistré, une fois téléchargé.
    #[serde(default)]
    pub saved_uri: Option<String>,
}

impl FileItem {
    pub(crate) fn from_info(f: &ot_core::FileInfo) -> Self {
        Self {
            blob_id: f.blob_id.to_string(),
            name: f.name.clone(),
            mime: f.mime.clone(),
            size: f.size,
            saved_uri: None,
        }
    }
}

pub(crate) struct Source(pub Arc<dyn FileSource>);

impl ot_core::ChunkSource for Source {
    fn size(&self) -> u64 {
        self.0.size()
    }
    fn read_at(&self, offset: u64, len: u32) -> Result<Vec<u8>, String> {
        self.0.read_at(offset, len).map_err(|e| e.to_string())
    }
}

pub(crate) struct Sink {
    pub inner: Arc<dyn FileSink>,
    /// URI rendue par `finish`.
    pub uri: std::sync::Mutex<Option<String>>,
}

impl ot_core::ChunkSink for Sink {
    fn write_at(&self, offset: u64, data: &[u8]) -> Result<(), String> {
        self.inner.write_at(offset, data.to_vec()).map_err(|e| e.to_string())
    }
    fn finish(&self) -> Result<(), String> {
        let uri = self.inner.finish().map_err(|e| e.to_string())?;
        *self.uri.lock().unwrap_or_else(|e| e.into_inner()) = Some(uri);
        Ok(())
    }
    fn abort(&self) {
        self.inner.abort();
    }
}

pub(crate) struct Listener(pub Arc<dyn TransferListener>);

impl ot_core::Progress for Listener {
    fn progress(&self, done: u64, total: u64) {
        self.0.on_progress(done, total);
    }
}
