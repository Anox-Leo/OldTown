# Décisions

Une note par décision qui s'écarte des plans de `docs/` ou qui les précise.
Le code fait foi ; les plans des étapes suivantes partent de ces notes.

Format : contexte, décision, conséquences. Une décision remplacée n'est pas
effacée : on la marque « Remplacée par NNNN ».

| # | Décision | Étape | Statut |
| --- | --- | --- | --- |
| [0001](0001-trames-liste-appareils.md) | Trames `list_devices` et `devices` | 1 | Acceptée |
| [0002](0002-echec-auth-ferme.md) | Un échec d'authentification ferme la connexion | 1 | Acceptée |
| [0003](0003-sqlx-requetes-dynamiques.md) | sqlx sans requêtes vérifiées à la compilation | 1 | Acceptée |
| [0004](0004-metriques-maison.md) | Compteurs Prometheus écrits à la main | 1 | Acceptée |
| [0005](0005-service-utilisateur-ot-relay.md) | Service sous `ot-relay` plutôt que `DynamicUser` | 1 | Acceptée |
| [0006](0006-sauvegarde-appareils.md) | La sauvegarde ne contient que les appareils | 1 | Acceptée |
| [0007](0007-sqlite-begin-immediate.md) | Écritures SQLite en `BEGIN IMMEDIATE` | 1 | Acceptée |
| [0008](0008-rust-1-88.md) | Rust 1.88 minimum | 1 | Acceptée |
| [0009](0009-une-session-par-appareil.md) | Une session par appareil : conséquences pour la CLI | 1 | Acceptée |
| [0010](0010-depot-public-mit.md) | Monorepo, licence MIT, publication après l'étape 2 | 1 | Publication remplacée par 0011 |
| [0011](0011-publication-fin-de-projet.md) | Publication à la fin du projet | 1 | Acceptée |
