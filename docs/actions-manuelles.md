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

### 1. Relais : mise à jour et TLS (laptop Ubuntu) — étape 3, T8 et T9

Depuis la copie du dépôt à jour, sur le laptop :

1. Compiler et installer le relais et les unités :

   ```sh
   cargo build --release --locked -p ot-server
   sudo install -m 0755 target/release/ot-server /usr/local/bin/
   sudo install -m 0644 deploy/ot-server.service deploy/ot-tls-renew.service deploy/ot-tls-renew.timer /etc/systemd/system/
   sudo install -D -m 0644 README.md /usr/local/share/doc/oldtown/README.md
   sudo systemctl daemon-reload
   ```

2. Dans la console d'administration Tailscale : *DNS* → activer *HTTPS
   Certificates* (si ce n'est pas déjà fait).

3. Nom complet du laptop dans le tailnet, puis premier certificat :

   ```sh
   sudo install -d /etc/oldtown
   # Nom complet du laptop, lu par Tailscale (sans le point final) : rien à recopier.
   echo "OT_TLS_DOMAIN=$(tailscale status --json | jq -r '.Self.DNSName | rtrimstr(".")')" | sudo tee /etc/oldtown/tls.env
   # doit afficher OT_TLS_DOMAIN=<nom>.<tailnet>.ts.net
   sudo systemctl start ot-tls-renew.service
   sudo ls -l /var/lib/ot-relay/tls/                   # relais.crt et relais.key, à ot-relay
   sudo systemctl enable --now ot-tls-renew.timer
   ```

   Si `jq` manque : `sudo apt install jq`, ou lire le nom dans
   `tailscale status`.

4. Activer TLS dans la surcharge du service (`sudo systemctl edit
   ot-server`), sous la ligne `OT_LISTEN` existante :

   ```ini
   Environment=OT_TLS_CERT=/var/lib/ot-relay/tls/relais.crt
   Environment=OT_TLS_KEY=/var/lib/ot-relay/tls/relais.key
   ```

   ```sh
   sudo systemctl restart ot-server
   journalctl -u ot-server -n 5                        # « relais démarré » avec scheme=wss
   curl https://<nom complet>:7878/healthz             # ok
   ```

   À lancer sur le laptop (ou tout appareil du tailnet). `<nom complet>` est
   celui écrit dans `/etc/oldtown/tls.env` (`<nom>.<tailnet>.ts.net`) : le
   certificat ne vaut que pour ce nom, ni pour l'IP ni pour le nom court. Si
   le laptop n'utilise pas le DNS de Tailscale :
   `curl --resolve <nom complet>:7878:$(tailscale ip -4) https://<nom complet>:7878/healthz`.

   À partir d'ici, les CLI en `ws://` ne se connectent plus : faire le lot 2
   tout de suite.

### 2. CLI du Mac et du PC Windows : nouvelle version et URL en `wss` — étape 3

1. Obtenir la nouvelle CLI : après le push sur `main` (commit à faire de mon
   côté ou à me demander), artefacts `ot-macOS-ARM64` et `ot-Windows-X64` de
   la CI, installés comme au README (section « CLI ») ; ou, sur chaque
   machine avec Rust : `cargo install --locked --path crates/ot-cli`.
2. Sur chaque appareil :

   ```sh
   ot --version
   ot init --server wss://<nom complet>:7878
   ot devices                                          # connexion en TLS
   ```

### 3. Mac : Android Studio et chaîne Rust pour Android — étape 3, T3

1. Installer Android Studio (developer.android.com/studio), puis au premier
   lancement l'assistant standard (SDK compris).
2. Rust sur le Mac, s'il n'y est pas : `curl --proto '=https' --tlsv1.2 -sSf
   https://sh.rustup.rs | sh`, puis rouvrir le terminal.
3. Cibles Android et `cargo-ndk` :

   ```sh
   rustup target add aarch64-linux-android x86_64-linux-android
   cargo install --locked cargo-ndk
   ```

4. NDK : *Settings* → *Languages & Frameworks* → *Android SDK* → onglet
   *SDK Tools* → cocher *Show Package Details* → *NDK (Side by side)* →
   **29.0.14206865** → *Apply*. Gradle ne le télécharge pas seul pour le
   cœur Rust (`cargo ndk`).
5. Android Studio : *File* → *Open* → le dossier `apps/android` du dépôt.
   Accepter la synchronisation Gradle : elle télécharge la plateforme
   Android 37 si besoin. JDK de Gradle : celui d'Android Studio
   (*Settings* → *Build, Execution, Deployment* → *Build Tools* → *Gradle* →
   *Gradle JDK* : `jbr-21`).
6. Facultatif, dans `apps/android/local.properties` (créé par Android
   Studio, jamais versionné) :

   ```properties
   oldtown.abis=arm64-v8a
   ```

   (ne compile que pour le téléphone : build deux fois plus court). Si le
   build dit « cargo introuvable » : ajouter `cargo.dir=/Users/<moi>/.cargo/bin`.
7. Compiler : *Build* → *Generate App Bundles or APKs* → *Generate APKs*
   (ou `./gradlew assembleDebug` dans `apps/android`). Le premier build
   compile le cœur Rust (quelques minutes). APK :
   `apps/android/app/build/outputs/apk/debug/app-debug.apk`.

### 4. Téléphone : installation et appairage — étape 3

1. Tailscale installé et connecté sur le téléphone ; vérifier dans un
   navigateur du téléphone : `https://<nom complet>:7878/healthz` → `ok`.
2. Installer l'APK : téléphone branché en USB avec le débogage USB activé
   (*Paramètres* → *À propos* → toucher 7 fois *Numéro de build*, puis
   *Options pour les développeurs* → *Débogage USB*), puis *Run* dans
   Android Studio, ou :

   ```sh
   ~/Library/Android/sdk/platform-tools/adb install -r apps/android/app/build/outputs/apk/debug/app-debug.apk
   ```

3. Sur le Mac : `ot pair new --qr`. Sur le téléphone : ouvrir OldTown, nom
   (ex. `android` ; il ne doit pas être déjà pris sur le relais), *Scanner le
   QR*, autoriser la caméra.
4. Accepter les notifications, puis le bandeau *Optimisation de batterie* →
   *Corriger* → autoriser.
5. Vérifier : sur le Mac, `ot group show` liste le téléphone ; dans l'appli,
   *Groupe* montre les mêmes empreintes.

Garder la même clé de signature d'APK ensuite : passer d'un APK de debug à
un APK signé autrement oblige à désinstaller l'appli, donc à la réappairer.

### 5. Essai réel (critères de fin de l'étape 3) — étape 3, T11

Noter les résultats et me les transmettre (je remplirai le tableau de
`docs/etape-3.md`) :

1. Chrome sur le téléphone → *Partager* → OldTown → `mac` ; sur le Mac,
   `ot listen` affiche le lien (moins de 2 s ?). Essayer aussi le raccourci
   Direct Share « mac » proposé en haut du menu *Partager*.
2. Téléphone verrouillé, sur le Mac : `ot send "code 4821"` → notification ;
   *Copier* → coller ailleurs sur le téléphone.
3. QR inversé : sur le laptop,
   `ot --home /tmp/ot-essai pair join --qr --server wss://<nom complet>:7878 --name essai` ;
   sur le téléphone, *Groupe* → *Ajouter un appareil* → scanner → *Ajouter* ;
   le terminal affiche « Groupe rejoint ». Nettoyer ensuite : dans l'appli,
   *Groupe* → corbeille sur `essai`, puis sur le laptop
   `ots device revoke essai` et `rm -rf /tmp/ot-essai`.
4. Une nuit en veille : noter le pourcentage de batterie le soir et le
   matin, et envoyer quelques messages pendant la nuit depuis le Mac (ils
   doivent être là au réveil, sans doublon). Consommation de l'appli :
   *Paramètres* → *Batterie*.
5. Redémarrer le téléphone sans ouvrir l'appli : la notification
   « Connecté » revient seule, et un `ot send` arrive.
6. Passer du Wi-Fi à la 4G (couper le Wi-Fi) : un `ot send` arrive en moins
   de 10 s.
