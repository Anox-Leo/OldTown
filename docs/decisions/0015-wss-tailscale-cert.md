# 0015 — `wss` dès l'étape 3, avec un certificat `tailscale cert`

**Statut : acceptée** (10 octobre 2026). Tranche D13 de
`decisions-a-prendre.md`.

**Contexte.** Le relais parle `ws://` sur Tailscale ; le plan de l'étape 3
gardait `ws://` pour Android et repoussait `wss` à la liste avant publication.
À terme, je veux pouvoir me passer de Tailscale et joindre le relais par ma
box. L'extension Chrome (note 0017) parle aussi au relais depuis un
navigateur, qui n'accepte pas un certificat auto-signé sans l'installer sur
chaque machine.

**Décision.**

- Le relais sert en TLS (`wss`) à partir de l'étape 3 ; Android et
  l'extension se connectent en `wss`. `ws://` reste possible pour les tests et
  le développement local (pas de certificat configuré).
- Certificat pour l'instant : `tailscale cert` (Let's Encrypt) pour le nom
  MagicDNS du laptop, renouvelé automatiquement (certificat de 90 jours).
- Le jour où le relais passe derrière la box : nouveau nom de domaine et
  nouveau certificat, sans changement de code. L'URL du relais doit donc
  rester un réglage modifiable dans chaque client.

**Conséquences.**

- Accepté : le nom de la machine et celui du tailnet (`*.ts.net`) sont publiés
  dans les journaux publics de Certificate Transparency.
- Ce nom ne va jamais dans le dépôt : il vit dans `private/` et dans la
  configuration locale (règle de la note 0011). Les exemples restent
  `relais`.
- Étape 3 : TLS côté serveur (`rustls`, rechargement du certificat
  renouvelé) et côté client (`ot-core`) ; minuterie de renouvellement sur le
  laptop, lancée par moi (`sudo`).
- Sortir de Tailscale exposera le relais à Internet : les protections de
  `SECURITY.md` (limites de connexions, freinage des échecs
  d'authentification) deviennent alors des préalables (étape 8e).
