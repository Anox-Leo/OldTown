# Plan — Étape 4 : desktop Windows, Mac et Linux

L'étape 4 livre l'appli de bureau : une icône dans la barre de menu (Mac) ou
la zone de notification (Windows, Ubuntu), qui reçoit les messages en
notification, envoie un texte, un lien ou le presse-papiers, et accepte le
glisser-déposer.

Références : jalon « 4. Desktop » de la [conception](design.md),
[revue après l'étape 1](revue-etapes-0-1.md), note 0009, décisions D12, D16,
D17 et D19 de [`decisions-a-prendre.md`](decisions-a-prendre.md).

**Niveau de détail** : plan de cadrage (D2). Au démarrage de l'étape, un agent
relit ce plan contre l'état du code et des étapes 2-3, complète les tâches et
note les écarts dans `decisions/` avant de coder.

**Prérequis** : étapes 2 et 3 terminées (`SecretStore`, roster, appairage QR).

## Objectif et périmètre

**Résultat attendu** : sur le Mac, un clic sur l'icône ouvre une petite
fenêtre avec les derniers éléments reçus et une zone d'envoi ; un raccourci
global envoie le presse-papiers courant au téléphone ; un lien reçu du
téléphone s'affiche en notification et s'ouvre d'un clic. Même chose sous
Windows 10 et Ubuntu.

**Inclus**

- Appli Tauri 2 dans `apps/desktop`, cœur `ot-core` appelé directement en
  Rust (pas d'UniFFI).
- Icône de barre de menu, fenêtre d'envoi et de réception, notifications.
- Envoi manuel du presse-papiers (menu et raccourci global) ; le presse-papiers
  live est l'étape 6.
- Glisser-déposer de texte et de liens sur la fenêtre (fichiers : étape 5).
- Clés dans le coffre de l'OS.
- Socket locale, propre à l'utilisateur, qui servira à l'extension (étape 7)
  et, en option, à la CLI.
- Appairage : afficher un QR pour le téléphone, ou rejoindre avec un code.
- Une seule instance, démarrage avec la session.

**Exclus** : fichiers (5), presse-papiers live (6), extension (7), mise à jour
automatique et signature payante (8 ou jamais).

## Architecture

```text
┌────────────── Appli Tauri (une seule instance) ──────────────┐
│ Interface web (fenêtre) ⇄ commandes Tauri ⇄ cœur Rust         │
│                                         ├─ ot-core ──▶ relais │
│                                         ├─ SecretStore (coffre OS)
│                                         └─ socket locale ◀── extension, CLI
└───────────────────────────────────────────────────────────────┘
```

- L'appli est l'unique propriétaire de la connexion sur la machine (0009).
  La CLI installée à côté est un appareil distinct (D12), ou passe par la
  socket locale si l'option est retenue plus tard.
- **Interface** : Angular (D19), que je connais ; la fenêtre reste petite
  (liste des derniers éléments, zone d'envoi, choix du destinataire, état,
  réglages, groupe).
- **Coffre de l'OS** : crate `keyring` (Trousseau macOS, Gestionnaire
  d'identifiants Windows, Secret Service sous Linux). Le coffre garde une clé
  de chiffrement ; les secrets chiffrés vont dans le dossier de données de
  l'appli, parce que Windows limite la taille d'un identifiant.
- **Socket locale** : crate `interprocess` (socket Unix en `0600` dans le
  dossier de l'utilisateur, tube nommé Windows limité à l'utilisateur
  courant). Protocole : une ligne JSON par requête (`send`, `status`,
  `devices`), version en tête. Aucune clé ne sort par la socket.

## Plugins Tauri prévus

| Besoin | Plugin ou crate |
| --- | --- |
| Icône et menu | API `tray` de Tauri |
| Notifications | `tauri-plugin-notification` |
| Écrire le presse-papiers | `tauri-plugin-clipboard-manager` |
| Ouvrir un lien | `tauri-plugin-opener` |
| Raccourci global | `tauri-plugin-global-shortcut` |
| Une seule instance | `tauri-plugin-single-instance` |
| Démarrage avec la session | `tauri-plugin-autostart` |

Vérifier au démarrage de l'étape que chaque plugin couvre les trois OS ; les
actions dans les notifications ne sont pas homogènes (un clic qui ouvre la
fenêtre suffit partout).

## Particularités par OS

| OS | Point d'attention |
| --- | --- |
| macOS | Pas d'icône dans le Dock (`LSUIElement`) ; autorisation des notifications ; appli non signée : ouverture par clic droit → Ouvrir, ou signature ad hoc |
| Windows 10 | Avertissement SmartScreen pour une appli non signée ; identifiant d'application pour les notifications toast |
| Ubuntu (GNOME) | L'icône passe par l'extension AppIndicator (installée par défaut sur Ubuntu) ; paquet `.deb` ou AppImage |

## Découpage en tâches

- [ ] **T1. Squelette Tauri et CI.** `apps/desktop/` (Tauri 2 + Angular),
  `apps/desktop/CLAUDE.md`, workflow `.github/workflows/desktop.yml` filtré
  sur `apps/desktop/**` et `crates/**` : build Linux en PR, paquets des trois
  OS sur `main`. Fini quand : l'appli vide s'installe sur les trois OS.
- [ ] **T2. `SecretStore` du coffre OS.** Tests : aller-retour sur chaque OS en
  CI (Secret Service sous Linux avec un trousseau de test, ou test ignoré
  documenté).
- [ ] **T3. Cœur : client, événements, état.** Démarrage de `ot-core`, relais
  des événements vers l'interface, persistance des `msg_id` vus et des 50
  derniers éléments.
- [ ] **T4. Icône, fenêtre et notifications.** Menu : ouvrir, envoyer le
  presse-papiers, état, quitter. Notification à la réception ; clic → fenêtre.
- [ ] **T5. Envoi.** Zone de texte, choix du destinataire, glisser-déposer de
  texte et de liens, raccourci global « envoyer le presse-papiers ».
- [ ] **T6. Appairage et groupe.** Afficher un QR (parrain), coller un code
  (rejoindre), liste des membres, rotation.
- [ ] **T7. Socket locale.** Serveur de socket, protocole JSON versionné,
  tests d'intégration (un client local envoie, l'appli relaie). Note de
  décision sur le format.
- [ ] **T8. Une instance, démarrage auto, réglages.**
- [ ] **T9. Essai réel et documentation.** README (installation par OS),
  état de l'étape, notes de décision.

Chaque tâche finit par les validations Rust de `CLAUDE.md`, plus le lint et
les tests de l'interface (`npm run lint`, `npm test` dans `apps/desktop`).

## Critères de fin

| Scénario | Attendu | Vérifié par |
| --- | --- | --- |
| Téléphone → Mac, lien | Notification ; clic ouvre le lien | Essai réel |
| Raccourci sur Windows | Le presse-papiers arrive sur le téléphone | Essai réel |
| Glisser un texte sur la fenêtre | Envoyé au destinataire choisi | Essai réel |
| Deux lancements | Une seule instance, la seconde ouvre la fenêtre | Test + essai |
| Socket locale | Un client local envoie ; un autre utilisateur ne peut pas s'y connecter | Test d'intégration |
| Clés | Dans le coffre de l'OS, jamais en clair sur disque | Revue + test |
| Ubuntu | L'appli tourne et reçoit, sans presse-papiers live | Essai réel |
| CI | Paquets des trois OS construits sur `main` | CI |

## Points à trancher au démarrage

- Frontend Angular ou plus léger (D19).
- CLI : appareil distinct ou passage par la socket locale (D12).
- Signature ad hoc sur macOS, ou rien.
