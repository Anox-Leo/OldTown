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

### 1. Étape 5 : fin des vérifications (notes 0026 à 0030)

Le relais de dépannage (le Mac), l'extension et l'appli sont à jour. Restent :

1. **Node** (sur le Mac), pour `npm run deploy` et les builds de
   l'extension :

   ```sh
   brew upgrade node
   node --version                                              # v24.15 ou plus
   ```

2. **Tests instrumentés Android**, téléphone branché, avec une APK construite
   après le correctif du 14 octobre (les tests qui oublient l'appareil sont
   désormais sautés sur un téléphone appairé) :

   ```sh
   cd apps/android
   ./gradlew assembleDebug
   adb install -r app/build/outputs/apk/debug/app-debug.apk
   ./gradlew connectedDebugAndroidTest
   ```

3. **Essais non faits** : une vidéo d'environ 500 Mo du téléphone vers le
   Mac avec une coupure du Wi-Fi au milieu (la reprise est automatique) ;
   une photo du Mac vers le téléphone (*Téléchargements/OldTown*, puis
   *Ouvrir*).

4. **Quand le laptop Ubuntu sera réparé** : y déployer le relais (README,
   « Mise à jour », puis `df -h /var/lib/ot-relay` : 25 Go libres au moins
   pour le quota de 20 Gio), puis repointer les appareils vers lui
   (`ot init --server wss://<nom du relais>:7878` sur la CLI, URL du relais
   dans les réglages de l'extension et de l'appli).
