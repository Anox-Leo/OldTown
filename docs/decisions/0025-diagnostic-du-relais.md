# 0025 — Diagnostic du relais envoyé par un membre dédié

**Statut : acceptée** (12 octobre 2026). Hors des jalons : petit service
d'exploitation, sans changement du protocole ni des crates.

**Contexte.** Le laptop qui héberge le relais doit envoyer au téléphone un
bilan de deux lignes : alimentation avec niveau et état de la batterie (lus
dans `/sys`, comme le fait `upower`), et erreurs disque vues par le noyau. Il l'envoie à 00:00 et 12:00, et tout de suite quand
l'alimentation passe sur batterie ou revient sur secteur. Le relais ne
fabrique ni ne lit jamais de `payload` : l'envoyeur doit donc être un client
du groupe.

**Décision.**

- Le serveur porte un membre du groupe dédié, `relais-diag`, sous un
  utilisateur système `ot-diag` (sans shell ni `sudo`, clés dans
  `/var/lib/ot-diag/ot` en `0700`, illisibles par `ot-relay`). Il rejoint le
  groupe par QR inversé, parrainé par le téléphone.
- Envoi seul : `ot send` ouvre une session courte, comme tout appareil, en
  `wss` vers le nom complet du relais. Aucune autre identité n'utilise
  `relais-diag`, donc la règle d'une session par appareil (note 0009) ne
  coupe rien. Un verrou (`flock`) empêche deux envois simultanés du service.
- Un script shell (`deploy/ot-diag.sh`), une unité modèle
  `ot-diag@.service` (`report`, `power`), un timer et une règle udev sur les
  sources d'alimentation (`Mains`, `USB`). Le destinataire est réglé hors du
  dépôt, dans `/etc/oldtown/diag.env` (`OT_DIAG_TO`).
- Erreurs disque : motifs du noyau (`I/O error`, erreurs `nvme`/`ata`,
  erreurs de système de fichiers) dans le journal du noyau sur 12 h, tous
  démarrages confondus (`_TRANSPORT=kernel`, pas `-k` qui s'arrête au
  dernier redémarrage ; journal persistant requis), grâce au groupe
  `systemd-journal`. Un journal illisible est signalé comme tel, pas
  comme « aucune alerte ». Pas de SMART : `smartctl` demande root ou le
  groupe `disk` (lecture brute des disques).
- Événement d'alimentation : attente de 30 s pour que l'état se stabilise,
  envoi seulement si la source a changé depuis le dernier envoi (le rapport
  périodique la retient aussi), au plus un envoi toutes les 10 minutes, et
  toujours le dernier état. Un échec d'envoi fait réessayer systemd sans
  avancer l'état retenu.
- Alternatives écartées : écrire le message dans la base du relais ou passer
  par une socket locale d'`ot-server` (le relais fabriquerait du contenu, et
  un second chemin de protocole apparaîtrait) ; réutiliser une identité
  existante (elle couperait la session de l'appareil qui l'utilise).

**Conséquences.**

- La machine du relais porte une clé de groupe : qui en prend le contrôle
  (root, compte personnel avec `sudo`, vol sans disque chiffré) lit tout le
  contenu, pas seulement les métadonnées. Une faille d'`ot-server` reste
  limitée aux métadonnées. Accepté tant que le relais est aussi la machine
  de développement, qui porte déjà des clés du groupe ; à revoir avant un
  hébergement dédié (étape 8f). En cas de doute :
  `ot group rotate --exclude relais-diag` puis `ots device revoke relais-diag`.
- La CLI doit être installée dans `/usr/local/bin` : l'unité ne voit pas les
  dossiers personnels (`ProtectHome`).
- Les motifs d'erreurs disque peuvent manquer un message inédit ; les tests
  (`deploy/tests/ot-diag-test.sh`) fixent les avertissements bénins qui ne
  doivent pas déclencher d'alerte.
