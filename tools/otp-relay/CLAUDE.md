# otp-relay — consignes

Démon Python qui surveille une boîte IMAP en IDLE et publie les codes à usage
unique sur un socket Unix (notes 0045, 0048). Indépendant des crates : seul
`ot otp-bridge` (`crates/ot-cli/src/otp.rs`) en dépend, par le protocole du
socket.

- Python ≥ 3.14, **bibliothèque standard seule** : pas de dépendance sans
  note de décision.
- `PROTOCOL.md` est le contrat avec le pont. Un changement incompatible
  augmente `PROTOCOL_VERSION` (`otp_relay/__init__.py`) et `PROTOCOL` dans
  `otp.rs`, avec une note de décision.
- Jamais de code en clair dans le journal, sauf `log.show_codes = true`
  (choix explicite de l'utilisateur).
- `sender_verified` ne vient que de l'en-tête `Authentication-Results` de
  `mx.google.com` : ne jamais faire confiance à un autre en-tête sans
  rendre son émetteur réglable.
- Tests avec le code : `python3 -m unittest discover -s tests -t .`.
