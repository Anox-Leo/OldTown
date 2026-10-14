//! Compteurs du relais, exposés au format texte Prometheus sur `/metrics`.

use std::fmt::Write;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

#[derive(Debug, Default)]
pub struct Metrics {
    pub connections: AtomicU64,
    pub auth_failures: AtomicU64,
    pub messages_received: AtomicU64,
    pub messages_delivered: AtomicU64,
    pub messages_acked: AtomicU64,
    pub purged_acked: AtomicU64,
    pub purged_expired: AtomicU64,
    pub pairings_created: AtomicU64,
    pub pairings_redeemed: AtomicU64,
    /// Appareils enregistrés par un parrain (QR inversé).
    pub pairings_registered: AtomicU64,
    pub pairings_rejected: AtomicU64,
    /// Requêtes HTTP (fichiers) dont la signature est refusée.
    pub http_auth_failures: AtomicU64,
    pub blobs_created: AtomicU64,
    /// Fichiers refusés : trop gros ou quota atteint.
    pub blobs_rejected: AtomicU64,
    pub blob_chunks_uploaded: AtomicU64,
    pub blob_chunks_downloaded: AtomicU64,
    /// Fichiers supprimés : retirés par tous, annulés, purgés.
    pub blobs_done: AtomicU64,
    pub blobs_cancelled: AtomicU64,
    pub blobs_purged: AtomicU64,
}

impl Metrics {
    pub fn inc(counter: &AtomicU64) {
        counter.fetch_add(1, Relaxed);
    }

    pub fn render(&self, pending: u64, blob_bytes: u64) -> String {
        let mut out = String::new();
        let mut metric = |name: &str, kind: &str, help: &str, values: &[(&str, u64)]| {
            let _ = writeln!(out, "# HELP {name} {help}\n# TYPE {name} {kind}");
            for (labels, v) in values {
                let _ = writeln!(out, "{name}{labels} {v}");
            }
        };
        let get = |c: &AtomicU64| c.load(Relaxed);
        metric(
            "ot_connections",
            "gauge",
            "Sessions authentifiées ouvertes.",
            &[("", get(&self.connections))],
        );
        metric(
            "ot_auth_failures_total",
            "counter",
            "Authentifications refusées.",
            &[("", get(&self.auth_failures))],
        );
        metric(
            "ot_messages_received_total",
            "counter",
            "Messages acceptés et stockés.",
            &[("", get(&self.messages_received))],
        );
        metric(
            "ot_messages_delivered_total",
            "counter",
            "Trames deliver envoyées, rejeux compris.",
            &[("", get(&self.messages_delivered))],
        );
        metric(
            "ot_messages_acked_total",
            "counter",
            "Livraisons acquittées.",
            &[("", get(&self.messages_acked))],
        );
        metric(
            "ot_messages_pending",
            "gauge",
            "Livraisons en attente d'acquittement.",
            &[("", pending)],
        );
        metric(
            "ot_messages_purged_total",
            "counter",
            "Messages supprimés par la purge.",
            &[
                ("{reason=\"acked\"}", get(&self.purged_acked)),
                ("{reason=\"expired\"}", get(&self.purged_expired)),
            ],
        );
        metric(
            "ot_pairings_total",
            "counter",
            "Appairages : jetons créés et utilisés, appareils enregistrés par un parrain, refus.",
            &[
                ("{result=\"created\"}", get(&self.pairings_created)),
                ("{result=\"redeemed\"}", get(&self.pairings_redeemed)),
                ("{result=\"registered\"}", get(&self.pairings_registered)),
                ("{result=\"rejected\"}", get(&self.pairings_rejected)),
            ],
        );
        metric(
            "ot_http_auth_failures_total",
            "counter",
            "Requêtes HTTP de fichiers refusées à l'authentification.",
            &[("", get(&self.http_auth_failures))],
        );
        metric(
            "ot_blobs_total",
            "counter",
            "Fichiers : créés, refusés (taille, quota), supprimés (retirés, annulés, purgés).",
            &[
                ("{result=\"created\"}", get(&self.blobs_created)),
                ("{result=\"rejected\"}", get(&self.blobs_rejected)),
                ("{result=\"done\"}", get(&self.blobs_done)),
                ("{result=\"cancelled\"}", get(&self.blobs_cancelled)),
                ("{result=\"purged\"}", get(&self.blobs_purged)),
            ],
        );
        metric(
            "ot_blob_chunks_total",
            "counter",
            "Morceaux de fichiers déposés et retirés.",
            &[
                ("{dir=\"up\"}", get(&self.blob_chunks_uploaded)),
                ("{dir=\"down\"}", get(&self.blob_chunks_downloaded)),
            ],
        );
        metric(
            "ot_blob_bytes",
            "gauge",
            "Octets chiffrés réservés par les fichiers en transit.",
            &[("", blob_bytes)],
        );
        out
    }
}
