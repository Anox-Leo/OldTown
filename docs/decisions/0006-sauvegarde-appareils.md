# 0006 — La sauvegarde ne contient que les appareils

**Contexte.** T9 demande une sauvegarde de la base. À l'étape 1, les messages
y sont en clair, et la conception prévoit de sauvegarder « la configuration et
les clés du serveur, pas des messages ».

**Décision.** `deploy/ot-backup.sh` (timer quotidien) exporte la seule table
`devices` en `INSERT` SQL rejouables sur une base neuve, et garde 14 fichiers.

**Conséquences.** Perdre la base fait perdre les messages en attente, jamais
les appareils. Les messages ne sont jamais copiés ailleurs que dans la base.
