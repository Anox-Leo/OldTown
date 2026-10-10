//! Cœur client OldTown : identité d'appareil, connexion au relais,
//! authentification, reconnexion, file d'envoi et dédoublonnage.
//!
//! L'API est volontairement simple (un handle [`Client`] clonable et un flux
//! d'[`Event`]) pour être exposée plus tard via UniFFI et Tauri.

mod client;
mod dedup;
mod identity;

pub use client::{Accepted, Backoff, Client, ClientConfig, ClientError, Event, Events, Received};
pub use dedup::Dedup;
pub use identity::Identity;
pub use ot_proto::{self as proto, Content, ContentKind, DeviceId, DeviceInfo, ErrorCode, Ulid};
