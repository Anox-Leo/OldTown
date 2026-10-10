# 0013 — Appairage : code long à l'étape 2, QR inversé à l'étape 3

**Statut : acceptée** (10 octobre 2026). Tranche D5 de
`decisions-a-prendre.md`.

**Contexte.** L'étape 2 prévoit un code d'appairage de 46 caractères, affiché
par le parrain et saisi sur le nouvel appareil ; le QR de l'étape 3 devait
porter le même code. Ce sens ne marche bien que si le nouvel appareil a une
caméra : un PC qui rejoint le groupe devrait taper 46 caractères lus sur le
téléphone. Un code court n'est pas une solution sans PAKE (SPAKE2) : le relais
voit la demande chiffrée sous le secret et pourrait essayer toutes les valeurs
hors ligne.

**Décision.**

- **Étape 2 : code long, inchangé.** Il suffit pour la CLI : copier-coller
  dans une session SSH ou par un canal de confiance.
- **Étape 3 : appairage par le téléphone dans les deux sens.**
  - Le téléphone **rejoint** le groupe en scannant le QR du code long
    (`ot pair new --qr`), comme prévu.
  - Le téléphone, membre permanent, **parraine** les autres appareils par
    **QR inversé** : le nouvel appareil affiche un QR, le téléphone le scanne.
- **Pas de SPAKE2** pour l'instant. Le seul cas non couvert est un PC sans
  copier-coller et sans téléphone à portée : on tape le code long.

**QR inversé : principes** (format exact à figer à l'étape 3, avec golden,
vecteurs et note) :

1. Le nouvel appareil B génère ses clés Ed25519 et X25519 et un secret `S` de
   16 octets, puis affiche un QR : nom, clés publiques, `S`, et une signature
   de B sur ces données (le relais n'enregistre toujours que des clés dont le
   possesseur est présent).
2. Le téléphone A scanne et demande au relais, dans sa session authentifiée,
   d'enregistrer B (nouvelle trame, du type `pair_register`). Aucune requête
   non authentifiée de plus sur le relais.
3. A envoie à B la clé de groupe et le roster, puis le roster à tous. L'envoi
   à B doit être à la fois **authentifié par `S`** (le relais ne peut pas
   fabriquer de faux groupe) et **scellé pour la clé X25519 de B** (une photo
   du QR ne suffit pas à lire la clé de groupe). La construction est à choisir
   à l'étape 3 parmi des formes standard (HKDF avec `S` en entrée).
4. B réessaie de s'authentifier jusqu'à être enregistré (au plus 10 minutes),
   puis reçoit l'envoi de A comme un message ordinaire.

**Conséquences.**

- Étape 3 : la tâche T9 couvre les deux sens ; changement de protocole
  (nouvelle trame côté relais, golden) et de format chiffré (vecteurs).
- CLI : `ot pair join --qr` affiche le QR en terminal, y compris en SSH.
- Étape 4 : l'appli desktop affiche ce QR pour être parrainée par le
  téléphone.
- Un PC peut aussi parrainer un autre PC avec le code long de l'étape 2.
