# 0001 — Trames `list_devices` et `devices`

**Contexte.** Le plan de l'étape 1 prévoit `ot devices` et `ot send --to <nom>`,
mais aucune trame ne permet à un client de connaître les autres appareils.

**Décision.** Deux trames s'ajoutent au protocole v1 : `list_devices` (client →
serveur, sans champ) et `devices` (serveur → client, liste de
`{id, name, online}`). Les appareils révoqués n'y figurent pas.

**Conséquences.** Le serveur révèle à un appareil authentifié les noms et la
présence des autres appareils du même relais, ce qui est sans enjeu tant qu'un
relais ne sert qu'à une personne. Le format est figé par
`crates/ot-proto/tests/golden/{list_devices,devices}.json`.
