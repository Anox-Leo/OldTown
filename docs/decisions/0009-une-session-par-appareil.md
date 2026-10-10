# 0009 — Une session par appareil : conséquences pour la CLI

**Statut : ouverte.**

**Contexte.** Comme prévu par T6, un appareil n'a qu'une session active, et la
plus récente remplace l'ancienne. La CLI ouvre une connexion par commande :
lancer `ot send` ou `ot devices` sur un appareil qui fait tourner `ot listen`
coupe ce dernier, qui se reconnecte après un court délai.

**Décision (provisoire).** Le comportement est gardé : il ne perd aucun message
(livraison au moins une fois et dédoublonnage) et les applis des étapes 3 et 4
n'ouvriront qu'une connexion par appareil.

**Options si cela gêne.** Autoriser plusieurs sessions par appareil, avec un
curseur de livraison par session ; ou faire passer les commandes ponctuelles
par le processus `ot listen` (socket locale).
