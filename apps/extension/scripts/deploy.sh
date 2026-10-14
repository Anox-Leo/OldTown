#!/usr/bin/env bash
# Construit l'extension et copie `dist/extension` dans le dossier chargé par
# le navigateur (mode développeur), puis il reste à *Actualiser* l'extension
# dans `brave://extensions`.
#
#   scripts/deploy.sh <dossier>        ou   OT_EXT_DIR=<dossier> scripts/deploy.sh
#   npm run deploy -- <dossier>
#
# Le contenu du dossier est remplacé : par sécurité, il doit être vide, ou
# contenir déjà une extension OldTown (un `manifest.json` « OldTown »).
set -euo pipefail

app="$(cd "$(dirname "$0")/.." && pwd)"
dest="${1:-${OT_EXT_DIR:-}}"
if [[ -z "$dest" ]]; then
  echo "usage : $0 <dossier de l'extension>   (ou OT_EXT_DIR=<dossier>)" >&2
  exit 2
fi

# La CLI Angular demande Node 24.15 au moins.
node_version="$(node --version | sed 's/^v//')"
IFS=. read -r major minor _ <<<"$node_version"
if (( major < 24 || (major == 24 && minor < 15) )); then
  echo "Node $node_version trop ancien : 24.15 au moins (brew upgrade node)." >&2
  exit 1
fi

# Une destination dans le dépôt est une erreur (ex. `npm run deploy --
# OT_EXT_DIR`, où le nom de la variable est pris pour un dossier) : refusée.
dest_abs="$(cd "$(dirname "$dest")" 2>/dev/null && pwd)/$(basename "$dest")" || dest_abs="$dest"
repo="$(cd "$app/../.." && pwd)"
if [[ "$dest_abs" == "$repo" || "$dest_abs" == "$repo"/* ]]; then
  echo "$dest est dans le dépôt : donner le dossier chargé par le navigateur." >&2
  echo "Avec OT_EXT_DIR défini, lancer simplement : npm run deploy" >&2
  exit 1
fi

if [[ -e "$dest" ]]; then
  if [[ ! -d "$dest" ]]; then
    echo "$dest existe et n'est pas un dossier." >&2
    exit 1
  fi
  if [[ -n "$(ls -A "$dest")" ]] && ! grep -qs '"name": "OldTown"' "$dest/manifest.json"; then
    echo "$dest n'est ni vide ni une extension OldTown : rien n'est remplacé." >&2
    exit 1
  fi
fi

cd "$app"
[[ -d node_modules ]] || npm ci
npm run build

mkdir -p "$dest"
rsync -a --delete "$app/dist/extension/" "$dest/"
echo "Extension copiée dans $dest : l'actualiser dans brave://extensions."
