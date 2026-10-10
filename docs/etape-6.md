# Plan — Étape 6 : presse-papiers

L'étape 6 rend le presse-papiers fluide entre mes appareils, toujours sur un
geste : j'envoie le presse-papiers d'un clic ou d'un raccourci, et l'appareil
qui reçoit peut l'écrire directement dans son presse-papiers. Rien ne part
jamais sans geste (note 0019).

Références : jalon « 6. Presse-papiers » de la [conception](design.md), notes
0016 (tuile Android), 0017 (extension Chrome, pas d'appli desktop) et 0019
(envoi manuel, écriture à la réception).

**Niveau de détail** : plan de cadrage (note 0012). Au démarrage, relire contre
le code (l'étape 4 envoie déjà le texte du presse-papiers), vérifier les API
de presse-papiers de Chrome et d'Android sur leur version courante, compléter
les tâches et noter les écarts dans `decisions/`.

**Prérequis** : étapes 3, 4 et 5 (les images passent par les blobs).

## Objectif et périmètre

**Résultat attendu** : je copie une phrase sur le Mac, j'appuie sur le
raccourci de l'extension ; sur le PC Windows, où l'écriture à la réception est
activée, Ctrl+V la colle. Je copie une capture d'écran sur le PC, je l'envoie
depuis la popup : elle se colle sur le Mac.

**Inclus** : contenu `clip` (texte, lien, image), envoi d'images depuis
l'extension, écriture automatique à la réception (réglage par appareil,
désactivé par défaut) dans l'extension et sur Android, marquage sensible,
tuile Android « envoyer le presse-papiers » si elle reste utile.

**Exclus** : toute surveillance automatique du presse-papiers (note 0019),
copie de fichiers (l'étape 5 suffit).

## Conception

**Contenu** : nouveau `kind: "clip"` dans le corps chiffré, avec `mime`
(`text/plain`, `text/uri-list`, `image/png`), `clip_id` (ULID) et `sensitive`.
Texte jusqu'à 48 Kio dans le message ; au-delà, et pour les images, un blob de
l'étape 5.

**Écriture à la réception** : extension par son document offscreen, Android
depuis le service (écriture permise en arrière-plan). Un contenu écrit à la
réception n'est jamais renvoyé automatiquement : pas d'anti-boucle à gérer.

**Sensible** : un envoi peut être marqué sensible (case dans la popup, option
dans la feuille de partage) ; il porte `sensitive: true`, n'est jamais écrit
automatiquement à la réception, Android l'écrit avec `EXTRA_IS_SENSITIVE`, et
il n'entre pas dans l'historique de l'étape 8.

## Découpage en tâches

- [ ] **T1. Modèle `clip` dans `ot-core` et dans le client TypeScript**,
  golden et vecteurs.
- [ ] **T2. Images depuis l'extension** : lecture d'une image du
  presse-papiers au clic, conversion en PNG, envoi par blob.
- [ ] **T3. Écriture à la réception** : réglage par appareil dans
  l'extension et sur Android, jamais pour un contenu sensible.
- [ ] **T4. Marquage sensible** de bout en bout.
- [ ] **T5. Tuile Android** (option, à confirmer au démarrage selon l'usage
  depuis l'étape 3).
- [ ] **T6. Essai réel croisé** et documentation.

## Critères de fin

| Scénario | Attendu | Vérifié par |
| --- | --- | --- |
| Raccourci sur le Mac, Ctrl+V sur Windows | Le texte se colle, en moins de 2 s | Essai réel |
| Image copiée et envoyée | Collable sur l'autre ordinateur | Essai réel |
| Contenu sensible reçu | Pas d'écriture automatique, pas d'historique | Test + essai |
| Copier sans envoyer | Rien ne part | Essai réel |
| Réglage désactivé | Le contenu arrive en notification seulement | Essai réel |
