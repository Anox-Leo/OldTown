# ot-proto

Types du protocole partagés par le relais et les clients : trames WebSocket
(`Frame`), enveloppe (`Envelope`, `Payload`), codes d'erreur, constantes
(version, tailles maximales, préfixe d'authentification).

- Aucune logique réseau, aucune E/S, aucune crypto de contenu : seulement des
  types, leur sérialisation et `auth_message`.
- **Le format est figé par `tests/golden/`.** Un test golden qui casse signale
  un changement de format : s'il est voulu, mettre à jour le fichier JSON,
  écrire une note dans `docs/decisions/` et prévenir que relais et clients
  doivent être déployés ensemble.
- Nouvelle trame : variante de `Frame`, entrée dans `KNOWN_TYPES` et
  `type_name`, fichier golden, test aller-retour.
- `deny_unknown_fields` partout : un champ inconnu est une erreur, pas un
  silence.
- Le contenu applicatif (`Content`) et le format chiffré de `payload.data`
  n'appartiennent pas au relais ; à partir de l'étape 2, le format chiffré
  est défini et figé dans `ot-crypto`.
