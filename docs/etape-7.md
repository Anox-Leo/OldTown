# Plan — Étape 7 : extension navigateur

L'étape 7 ajoute une extension pour Chrome et Firefox : envoyer la page, un
lien, une sélection ou une image en un clic droit, et voir les derniers
éléments reçus dans une popup. L'extension ne détient aucune clé et ne parle
jamais au relais.

Références : jalon « 7. Extension navigateur » de la
[conception](design.md), [revue après l'étape 1](revue-etapes-0-1.md)
(conséquence de la note 0009), décision D21 de
[`decisions-a-prendre.md`](decisions-a-prendre.md).

**Niveau de détail** : plan de cadrage (D2). Au démarrage, relire contre le
code, vérifier les règles Manifest V3 et de distribution du moment, compléter
les tâches et noter les écarts dans `decisions/`.

**Prérequis** : étape 4 (socket locale de l'appli desktop), étape 5 pour les
images.

## Objectif et périmètre

**Résultat attendu** : clic droit sur un lien dans Chrome sous Windows →
« Envoyer à OldTown » → « Téléphone » ; la notification arrive sur le
téléphone.

**Inclus** : extension Manifest V3 commune à Chrome et Firefox dans
`apps/extension`, menus contextuels (page, lien, sélection, image), popup
(derniers éléments, zone d'envoi, choix du destinataire), raccourci clavier,
hôte de native messaging, installation de son manifeste par l'appli desktop.

**Exclus** : extension sur le téléphone, Safari, publication sur les
boutiques.

## Conception

```text
Extension ──native messaging (stdio)──▶ ot-native-host ──socket locale──▶ appli desktop ──▶ relais
```

- **Pourquoi un hôte séparé** : le navigateur lance un **nouveau processus**
  hôte à chaque connexion. Si cet hôte ouvrait sa propre session, il couperait
  celle de l'appli (note 0009). Il se contente donc de relayer vers la socket
  locale de l'étape 4. Appli fermée : l'hôte répond « Ouvre OldTown ».
- **`ot-native-host`** : petit binaire Rust (`crates/ot-native-host`), messages
  préfixés par leur longueur (format du native messaging), traduits ligne à
  ligne vers le protocole JSON de la socket. Pas de clé, pas d'accès réseau.
- **Manifeste de l'hôte** : installé par l'appli desktop (clé de registre sous
  Windows, fichiers JSON dans les dossiers de Chrome et Firefox sous macOS et
  Linux), limité à l'identifiant fixe de l'extension (`allowed_origins` pour
  Chrome, `allowed_extensions` pour Firefox).
- **Extension** : TypeScript, API `browser.*` avec une couche de compatibilité
  pour Chrome ; permissions minimales (`contextMenus`, `nativeMessaging`,
  `activeTab`) ; aucun script injecté dans les pages.
- **Images** : l'URL part comme lien ; « Envoyer l'image » télécharge l'image
  dans l'extension et la passe à l'appli, qui en fait un fichier (étape 5).
- **Distribution** (D21) : Chrome en mode développeur (extension non
  empaquetée, identifiant fixé par la clé du manifeste) ; Firefox demande une
  signature de Mozilla même pour une extension non publiée (« unlisted ») —
  sinon Firefox Developer Edition avec la vérification de signature désactivée.

## Découpage en tâches

- [ ] **T1. `ot-native-host`** : protocole, relais vers la socket, tests
  d'intégration avec une fausse appli.
- [ ] **T2. Installation des manifestes** par l'appli desktop sur les trois OS,
  et désinstallation.
- [ ] **T3. Squelette de l'extension** et CI (`.github/workflows/extension.yml`
  filtré sur `apps/extension/**` : lint, tests, archive).
- [ ] **T4. Menus contextuels et raccourci.**
- [ ] **T5. Popup** : derniers éléments (demandés à l'appli), envoi, état.
- [ ] **T6. Images** via l'appli.
- [ ] **T7. Essai réel** Chrome et Firefox sur Windows et Mac, documentation.

## Critères de fin

| Scénario | Attendu | Vérifié par |
| --- | --- | --- |
| Clic droit sur un lien → téléphone | Notification sur le téléphone | Essai réel |
| Sélection de texte → Mac | Arrive sur le Mac | Essai réel |
| Appli desktop fermée | Message clair dans l'extension | Test + essai |
| Envoi depuis l'extension | La session de l'appli n'est pas coupée | Test d'intégration |
| Une autre extension ou une page | Ne peut pas joindre l'hôte | Revue du manifeste |
