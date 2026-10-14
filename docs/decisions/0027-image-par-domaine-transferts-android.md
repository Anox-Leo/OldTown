# 0027 — Images des pages autorisées domaine par domaine ; transferts Android dans le service

**Statut : acceptée** (13 octobre 2026). Tranche D25 et D26 de
`decisions-a-prendre.md`, ouvertes par la note
[0026](0026-relecture-etape-5.md).

**Contexte.** « Envoyer l'image » ne reçoit que l'URL de l'image ; quand elle
est servie par un autre domaine que la page (CDN, cas courant), CORS empêche
l'extension de la lire sans permission d'hôte. `<all_urls>` d'un bloc
permettrait à un code compromis (dépendance npm piégée, dossier de
l'extension modifié) de lire tout site avec ma session, et de voir l'adresse
de tous mes onglets. Côté Android, le plan prévoyait WorkManager pour les
longs transferts.

**Décision.**

- **D25 — Autorisation domaine par domaine.** Le manifeste déclare
  `<all_urls>` en `optional_host_permissions`, mais l'extension ne demande
  jamais que l'origine de l'image concernée (`https://img.example/*`), au
  premier échec sur ce domaine : « Autoriser OldTown à lire les images de
  img.example ? ». Les autorisations s'accumulent et se retirent dans les
  réglages du navigateur. Les requêtes vers les images se font sans cookies
  (`credentials: 'omit'`). Si Chromium refuse d'afficher la demande depuis
  le menu contextuel (geste utilisateur non reconnu), la demande passe par
  la popup ou une petite fenêtre de l'extension. Écartés : `<all_urls>`
  accordée d'un bloc (surface trop large, surtout sur un ordinateur
  professionnel) ; aucune permission (échec sur une bonne partie des sites).
- **D26 — Transferts dans `RelayService`**, déjà au premier plan
  (`specialUse`) et seul propriétaire du client Rust (note 0009), avec une
  notification de progression (canal « Transferts »). Pas de WorkManager.

**Conséquences.**

- L'extension ne peut lire que les domaines d'images que j'ai validés, plus
  l'origine du relais (note 0026).
- Une demande par nouveau domaine d'images : un peu de friction, assumée.
- Si le service Android est arrêté, les transferts s'arrêtent et reprennent
  à son redémarrage ; pas de limite de durée (`dataSync` évité).
