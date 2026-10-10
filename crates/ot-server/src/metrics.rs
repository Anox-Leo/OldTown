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
    pub pairings_rejected: AtomicU64,
}

impl Metrics {
    pub fn inc(counter: &AtomicU64) {
        counter.fetch_add(1, Relaxed);
    }

    pub fn render(&self, pending: u64) -> String {
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
            "Jetons d'appairage : créés, utilisés, refusés.",
            &[
                ("{result=\"created\"}", get(&self.pairings_created)),
                ("{result=\"redeemed\"}", get(&self.pairings_redeemed)),
                ("{result=\"rejected\"}", get(&self.pairings_rejected)),
            ],
        );
        out
    }
}
