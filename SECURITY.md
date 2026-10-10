# Sécurité

OldTown est un projet personnel en cours de construction. **Ne pas l'utiliser
pour des données sensibles avant l'étape 2**, ni exposer le relais sur Internet.

## Modèle de menace

| Acteur | Ce qu'il peut faire aujourd'hui (étape 1) | Cible |
| --- | --- | --- |
| Réseau entre appareil et relais | Rien : le relais n'écoute que sur Tailscale (WireGuard) | Option `wss` en plus (Q5) |
| Relais ou qui lit sa base | Lire les messages en attente : `payload` est en clair (`enc: "none"`) | Étape 2 : chiffrement de bout en bout, le relais ne voit que des enveloppes opaques |
| Appareil du tailnet non enregistré | Ouvrir des connexions, échouer à l'authentification | Limites de connexions et de débit |
| Appareil enregistré | Envoyer à tous les autres appareils, voir leurs noms et leur présence | Inchangé : un relais sert une seule personne |
| Appareil volé | Tout ce que fait un appareil enregistré, jusqu'à `ot-server device revoke` | Étape 8 : révocation avec rotation de la clé de groupe |

Ce qui est déjà en place :
- Authentification par défi Ed25519 signé, avec un préfixe de domaine
  (`ot-auth-v1`) et un nonce à usage unique.
- Taille des trames et des enveloppes bornée.
- Aucun contenu de message dans les logs.
- Service systemd sous un utilisateur dédié, durci.
- `cargo deny` en CI (failles connues, licences, sources).
- Clé privée de la CLI dans un fichier `0600`.

## Avant de rendre le dépôt public

- [ ] Étape 2 livrée : contenu chiffré de bout en bout, relais aveugle.
- [ ] Limiter les connexions non authentifiées (nombre simultané, délai) et
      freiner les échecs d'authentification.
- [ ] Option TLS (`wss`) pour un relais hors Tailscale, et avertissement clair
      dans le README.
- [ ] Relire la partie cryptographique (idéalement par quelqu'un d'autre).
- [ ] Vérifier que ni le dépôt ni son historique ne contiennent de données
      personnelles : `git log -p | grep -iE '<ip|nom de machine|tailnet>'`.
- [ ] Activer sur GitHub : détection de secrets avec blocage des pushs,
      Dependabot, protection de `main`, signalement privé de failles.

## Signaler une faille

Une fois le dépôt public : par le signalement privé de GitHub (onglet
*Security* → *Report a vulnerability*), pas par une issue publique.
