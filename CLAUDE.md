# OldTown — consignes pour les agents

Système maison d'échange de textes, liens, fichiers et presse-papiers entre mes
appareils (Android, Mac, Windows, navigateur), via un relais auto-hébergé
joignable par Tailscale. Réponses et documentation en français.

## Où regarder

- `docs/design.md` : conception et feuille de route (jalons 1 à 8).
- `docs/etape-N.md` : plan détaillé de l'étape N.
- `docs/decisions/` : écarts et précisions par rapport aux plans. **Le code et
  ces notes font foi** ; toute nouvelle déviation d'un plan donne une note.
- `SECURITY.md` : modèle de menace et conditions avant publication.

## Règles

- Le dépôt deviendra public : jamais de nom de machine, IP, nom de tailnet,
  chemin personnel ou secret dans les fichiers versionnés. Exemple de nom d'hôte :
  `relais`. Les réglages personnels vont dans `private/` (ignoré par git).
- `ot-server` ne dépend que de `ot-proto` et ne lit jamais `payload`.
- Le format du protocole est figé par `crates/ot-proto/tests/golden/` : un
  changement de format est délibéré et accompagné d'une note de décision.
- Jamais de contenu de message dans les logs.
- Avant de rendre la main : `cargo fmt --all`, `cargo clippy --workspace
  --all-targets -- -D warnings`, `cargo test --workspace`, `cargo deny check`.
- Commits au format Conventional Commits, seulement sur demande.
