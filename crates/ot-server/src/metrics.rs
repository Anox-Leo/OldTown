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
    /// Connexions et requêtes refusées par les limites (étape 8e).
    pub limited_busy: AtomicU64,
    pub limited_failures: AtomicU64,
    /// Trames et fichiers refusés par le réseau ou les quotas (étape 10).
    pub refused_forbidden: AtomicU64,
    pub refused_not_allowed: AtomicU64,
    pub refused_quota: AtomicU64,
    /// Invitations : créées, utilisées, refusées, certifiées.
    pub invitations_created: AtomicU64,
    pub invitations_redeemed: AtomicU64,
    pub invitations_rejected: AtomicU64,
    pub invitations_certified: AtomicU64,
}

impl Metrics {
    pub fn inc(counter: &AtomicU64) {
        counter.fetch_add(1, Relaxed);
    }

    pub fn render(
        &self,
        pending: u64,
        blob_bytes: u64,
        users: &[(ot_proto::Role, ot_proto::UserState, u64)],
    ) -> String {
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
        metric(
            "ot_limited_total",
            "counter",
            "Connexions et requêtes refusées : trop de connexions non authentifiées, trop d'échecs.",
            &[
                ("{reason=\"busy\"}", get(&self.limited_busy)),
                ("{reason=\"failures\"}", get(&self.limited_failures)),
            ],
        );
        metric(
            "ot_refused_total",
            "counter",
            "Trames et fichiers refusés : trame interdite, destinataire hors réseau, quota d'une personne.",
            &[
                ("{reason=\"forbidden\"}", get(&self.refused_forbidden)),
                ("{reason=\"not_allowed\"}", get(&self.refused_not_allowed)),
                ("{reason=\"quota\"}", get(&self.refused_quota)),
            ],
        );
        metric(
            "ot_invitations_total",
            "counter",
            "Invitations : créées, utilisées, refusées, certifiées.",
            &[
                ("{result=\"created\"}", get(&self.invitations_created)),
                ("{result=\"redeemed\"}", get(&self.invitations_redeemed)),
                ("{result=\"rejected\"}", get(&self.invitations_rejected)),
                ("{result=\"certified\"}", get(&self.invitations_certified)),
            ],
        );
        let labels: Vec<String> = users
            .iter()
            .map(|(role, state, _)| {
                format!("{{role=\"{}\",state=\"{}\"}}", role.as_str(), state.as_str())
            })
            .collect();
        let values: Vec<(&str, u64)> =
            labels.iter().zip(users).map(|(l, (_, _, n))| (l.as_str(), *n)).collect();
        metric("ot_users", "gauge", "Personnes, par rôle et par état.", &values);
        out
    }
}
