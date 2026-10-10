# 0005 — Service sous `ot-relay` plutôt que `DynamicUser`

**Contexte.** La tâche T9 prévoit `DynamicUser`. L'étape 0 a créé un
utilisateur système `ot-relay` propriétaire de `/var/lib/ot-relay`, et
l'administration se fait par `ot-server device …` sur la même base.

**Décision.** `User=ot-relay` avec `StateDirectory=ot-relay`, plus le
durcissement systemd prévu (`ProtectSystem=strict`, filtre d'appels système,
aucune capacité, etc.). L'adresse d'écoute se règle par une surcharge locale
(`systemctl edit`), pas dans l'unité versionnée.

**Conséquences.** `sudo -u ot-relay ot-server device add …` fonctionne
directement. Les agents qui tournent sous l'utilisateur de développement ne
peuvent pas lire la base.
