# Actions manuelles

Ce qui reste à faire à la main sur mes machines (relais, Mac, PC Windows,
téléphone…), étape par étape, dans l'ordre. Uniquement les actions : le
pourquoi est dans le plan de l'étape ou la note de décision citée.

Règles pour les agents :
- ajouter ici toute action que je dois faire moi-même, au moment où le
  changement qui l'exige est livré, avec les commandes exactes ;
- noms génériques uniquement (`relais`, `mac`, `windows`…), jamais de nom de
  machine, d'IP ni de secret (le dépôt deviendra public) ;
- une section par lot, numérotée dans l'ordre d'exécution, avec sa source
  (tâche ou PR) ;
- une fois le lot fait et confirmé par moi, le retirer : l'historique git
  garde la trace.

## À faire

### 1. Relais : mise à jour pour l'extension (laptop Ubuntu) — étape 4, T3

Le relais doit connaître la trame `ping` (note 0023) avant que l'extension
s'y connecte. Depuis la copie du dépôt à jour (branche de l'étape 4, ou
`main` une fois fusionnée) :

```sh
cargo build --release --locked -p ot-server
sudo install -m 0755 target/release/ot-server /usr/local/bin/
sudo systemctl restart ot-server
journalctl -u ot-server -n 3                 # « relais démarré », scheme=wss
```

Rien d'autre ne change (ni la base, ni la configuration, ni les CLI).

### 2. Téléphone : nouvel APK (pas de notification appli ouverte) — étape 4, T8b

Sur le Mac (où Android Studio et la chaîne Android sont installés), dans la
copie du dépôt à jour, téléphone branché :

```sh
cd apps/android
./gradlew assembleDebug
adb install -r app/build/outputs/apk/debug/app-debug.apk
```

Ou, après un push, l'artefact `oldtown-debug-apk` du workflow *Android*.

### 3. Construire l'extension — étape 4

Au choix :

- **Artefact de la CI** : après le push de la branche (à faire de mon côté
  ou à me demander), workflow *Extension* → artefact `oldtown-extension`.
- **Sur le laptop Ubuntu**, dans la copie du dépôt à jour :

  ```sh
  rustup target add wasm32-unknown-unknown
  cargo install --locked wasm-bindgen-cli --version 0.2.129
  cd apps/extension
  npm ci
  npm run build                              # résultat : dist/extension
  ```

  puis copier le dossier `dist/extension` sur le Mac et le PC Windows (clé
  USB, `scp`…).

Sur chaque ordinateur, ranger le dossier à un endroit stable (par exemple
`~/OldTown/extension` sur le Mac, `C:\Users\<moi>\OldTown\extension` sous
Windows) : Chrome le relit à chaque démarrage.

La clé qui fixe l'identifiant de l'extension a été créée dans
`private/extension-key.pem` (laptop, jamais versionnée). Elle ne sert qu'à
empaqueter un `.crx` : la garder avec mes autres secrets, sans urgence.

### 4. Installer et appairer l'extension (Mac, puis PC Windows) — étape 4, T9

Sur chaque ordinateur, Tailscale connecté, dans Brave (ou Chrome) :

1. `brave://extensions` (`chrome://extensions` dans Chrome) → *Mode
   développeur* (en haut à droite) →
   *Charger l'extension non empaquetée* → le dossier de l'étape 4.
   L'identifiant affiché doit être `fiblpidpjopjpinfgonlkblkonllpgfb`.
2. Épingler l'icône d'OldTown (pièce de puzzle → épingle).
3. Ouvrir la popup : URL du relais `wss://<nom complet>:7878` (le même nom
   que dans `/etc/oldtown/tls.env`), nom proposé (`Chrome Mac`,
   `Chrome Windows` ; `Brave Mac`… avec la nouvelle version) ou un autre,
   puis *Afficher un QR à scanner*.
4. Sur le téléphone : *Groupe* → *Ajouter un appareil*, viser le QR, vérifier
   le nom, confirmer. La popup doit passer à l'écran principal.
5. Mac seulement : *Réglages Système* → *Notifications* → *Brave Browser*
   (ou *Google Chrome*) → autoriser.

### 5. Essai réel de l'étape 4 — étape 4, T9

Dérouler, puis me transmettre le résultat de chaque ligne :

1. Sur le PC Windows, clic droit sur un lien → *Envoyer à OldTown* →
   téléphone : notification sur le téléphone ?
2. Sur le téléphone, *Partager* une page → *Chrome Mac* : notification
   Chrome sur le Mac ; un clic ouvre le lien dans un onglet ?
3. Même envoi avec la popup du Mac ouverte : pas de notification, l'élément
   apparaît dans la liste ? Puis avec l'appli Android ouverte au premier
   plan, un envoi depuis le Mac : pas de notification, l'élément dans la
   liste ?
4. Sélectionner un texte sur le Mac → clic droit → *Envoyer à OldTown* →
   *Chrome Windows* : arrive sur le PC ? *Copier* sur la notification
   remplit le presse-papiers ?
5. `Alt+Maj+S` sur un onglet : l'adresse part au dernier destinataire choisi ?
6. Laisser Chrome ouvert sans y toucher une heure, puis envoyer depuis le
   téléphone : reçu tout de suite ? (popup → l'état doit être « Connecté »)
7. Fermer Chrome, envoyer deux messages depuis le téléphone, rouvrir Chrome :
   les deux arrivent ?
8. Réglages de l'extension (clic droit sur l'icône → *Options*) : état
   « Connecté », version affichée.

### 6. Essai des retours d'essai (Brave) — étape 4, note 0024

Après avoir rechargé la nouvelle version de l'extension (`brave://extensions`
→ *Actualiser*) et réinstallé l'APK (lot 2) :

1. Brave au premier plan, popup fermée : un lien envoyé du téléphone s'ouvre
   dans un nouvel onglet, sans notification ; un texte ouvre la popup.
2. Brave en arrière-plan (autre appli devant) : notification ; un clic sur
   la notification d'un texte ouvre la popup (ou une petite fenêtre
   OldTown si Brave refuse d'ouvrir la popup) ; celle d'un lien ouvre le
   lien.
3. `Alt+Maj+V` : le presse-papiers part au dernier destinataire (coche brève
   sur l'icône). À vérifier en particulier : c'est la seule lecture du
   presse-papiers que je n'ai pas pu tester ici.
4. Popup : *Entrée* envoie, *Maj+Entrée* va à la ligne ; bouton presse-papiers
   à côté du bouton d'envoi ; icône de réglages ; le groupe est dans les
   réglages.
5. Téléphone : appli ouverte, un message reçu n'affiche pas de notification ;
   en ouvrant l'appli, les notifications de messages déjà affichées
   disparaissent ; appli fermée, cinq messages d'affilée → trois
   notifications au plus, les plus récentes.

