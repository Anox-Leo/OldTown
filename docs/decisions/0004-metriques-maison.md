# 0004 — Compteurs Prometheus écrits à la main

**Contexte.** Le plan cite la crate `metrics`. Son enregistreur est global au
processus et ne s'installe qu'une fois, ce qui empêche de vérifier les
compteurs de plusieurs serveurs lancés dans les mêmes tests.

**Décision.** Des compteurs atomiques propres à chaque instance du relais
(`crates/ot-server/src/metrics.rs`), rendus au format texte Prometheus sur
`/metrics`.

**Conséquences.** Aucune dépendance, et les tests vérifient les compteurs. Il
n'y a pas d'histogrammes ; à revoir s'il en faut.
