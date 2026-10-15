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

### 1. Relais de retour sur le laptop Ubuntu (une fois réparé)

Le relais tourne pour l'instant sur le Mac, lancé à la main.

1. Sur le laptop : déployer la version à jour du relais (README, « Mise à
   jour »), puis vérifier la place pour le quota de 20 Gio des fichiers :

   ```sh
   df -h /var/lib/ot-relay                                     # 25 Go libres au moins
   ```

2. Repointer les appareils vers lui : `ot init --server wss://<nom du
   relais>:7878` sur la CLI, URL du relais dans les réglages de l'extension
   et de l'appli.
3. Arrêter le relais de dépannage du Mac.

### 2. Essai des conversations sur le téléphone (note 0033)

Source : plan [`plan-android-conversations.md`](plan-android-conversations.md),
section « Fin ».

1. Installer l'appli à jour, téléphone branché (l'historique et l'appairage
   sont gardés) :

   ```sh
   cd apps/android
   ./gradlew assembleDebug
   adb install -r app/build/outputs/apk/debug/app-debug.apk
   ./gradlew connectedDebugAndroidTest
   ```

2. Depuis l'extension, envoyer un message à « Téléphone » : il apparaît dans
   la conversation du navigateur sur le téléphone. Envoyer un message à
   tous : il apparaît dans « Tous mes appareils ».
3. Répondre depuis la conversation du navigateur sur le téléphone : la
   réponse apparaît dans la conversation du téléphone côté extension.
4. Comparer les avatars : un même appareil a la même couleur des deux côtés.
5. Toucher la notification d'un message reçu (appli en arrière-plan) : elle
   ouvre sa conversation.
6. Partager un lien depuis Chrome vers OldTown : les appareils sont rangés
   du plus utilisé au moins utilisé, « Tous mes appareils » en tête.

