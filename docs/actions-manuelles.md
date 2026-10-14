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
