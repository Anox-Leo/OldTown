# 0007 — Écritures SQLite en `BEGIN IMMEDIATE`

**Contexte.** En mode WAL, une transaction ordinaire qui lit puis écrit échoue
aussitôt en `SQLITE_BUSY`, sans attendre, si une autre connexion a écrit entre
les deux. La purge périodique pendant un `send` déclenchait exactement ce cas :
l'envoi échouait en `internal`.

**Décision.** Les transactions d'écriture (`insert_message`, `revoke_device`,
`purge`) commencent par `BEGIN IMMEDIATE` et prennent le verrou d'écriture dès
le début ; `busy_timeout` (5 s) les fait attendre au lieu d'échouer.

**Conséquences.** Les écritures sont sérialisées, ce qui est sans effet à
l'échelle d'un relais personnel. Le test `concurrent_writes_do_not_fail` couvre
le cas.
