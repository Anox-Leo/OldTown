# 0009 — Une session par appareil : conséquences pour la CLI

**Statut : acceptée** (10 octobre 2026).

**Contexte.** Comme prévu par T6, un appareil n'a qu'une session active, et la
plus récente remplace l'ancienne. La CLI ouvre une connexion par commande :
lancer `ot send` ou `ot devices` sur un appareil qui fait tourner `ot listen`
coupe ce dernier, qui se reconnecte après un court délai.

**Décision.** Le comportement est gardé : il ne perd aucun message
(livraison au moins une fois et dédoublonnage). La connexion WebSocket sert
dans les deux sens : un appareil envoie et reçoit en même temps sur sa seule
session, donc des échanges croisés entre appareils ne se gênent pas. Deux
règles évitent qu'une application se coupe elle-même :

- sur chaque appareil, un seul processus tient la connexion ; les autres
  points d'entrée (menu Partager d'Android, raccourcis, extension) passent
  par lui au lieu d'ouvrir leur propre session ;
- les transferts de fichiers de l'étape 5 (HTTP par morceaux) s'authentifient
  sans enregistrer de session dans le registre : un envoi volumineux ne doit
  pas couper la réception.

**Conséquences.** Seule la CLI lancée à côté d'une application qui utilise la
même identité coupe encore cette dernière ; contournement : enregistrer la CLI
comme un appareil distinct (`ot --home`). Si ce cas gêne, la piste retenue est
une session « envoi seul » (le client annonce à l'`auth` qu'il ne reçoit pas,
et sa session ne remplace pas celle qui reçoit). Écartés : plusieurs sessions
qui reçoivent (livraison en double sur le même appareil) et le passage par une
socket locale vers `ot listen` (propre à la CLI, sockets Unix et tubes nommés
Windows).
