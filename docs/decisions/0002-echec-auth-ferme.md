# 0002 — Un échec d'authentification ferme la connexion

**Contexte.** Le plan dit à la fois « `auth_failed` coupe la session » et
« trois échecs consécutifs ferment la connexion ».

**Décision.** Le premier `auth_failed` ferme la connexion. Le délai de réponse
au défi (30 s) dépassé aussi. Une trame autre que `auth` avant l'authentification
reçoit `unexpected_frame` sans fermer.

**Conséquences.** Plus strict que le plan : un nonce ne sert qu'une fois et ne
peut pas être réessayé. Le client se reconnecte, avec le délai maximal après un
`auth_failed`. Il n'y a pas encore de limite du nombre de connexions par adresse
(voir `SECURITY.md`).
