// Fichiers reçus servis par le service worker (note 0029, voie A) : leur
// adresse dans l'extension, et les types que le navigateur affiche lui-même.

/** Préfixe des adresses de fichiers, interceptées par le service worker. */
export const FILE_PATH = '/f/';

/** Petits fichiers copiés (chiffrés) dès la réception (note 0018). */
export const AUTO_CACHE_MAX = 20 * 1024 * 1024;

/**
 * Types ouverts dans le lecteur du navigateur. Ni SVG ni HTML : servis sous
 * l'origine de l'extension, ils pourraient porter du script ; ils se
 * téléchargent.
 */
const VIEWABLE =
  /^(image\/(png|jpeg|gif|webp|avif|bmp)|application\/pdf|text\/plain|audio\/(mpeg|mp4|ogg|wav|webm|aac|flac)|video\/(mp4|webm|ogg|quicktime))$/;

export function isViewable(mime: string): boolean {
  return VIEWABLE.test(mime.toLowerCase());
}

/** Chemin d'un fichier reçu : `/f/<blob_id>/<nom>` (`?dl=1` : téléchargement). */
export function filePath(blobId: string, name: string, download = false): string {
  return `${FILE_PATH}${blobId}/${encodeURIComponent(name)}${download ? '?dl=1' : ''}`;
}

/** Adresse complète, dans l'extension. */
export function fileUrl(blobId: string, name: string, download = false): string {
  return chrome.runtime.getURL(filePath(blobId, name, download).slice(1));
}

/** `blob_id` et téléchargement d'après une adresse `/f/…`, ou `undefined`. */
export function parseFilePath(url: URL): { blobId: string; download: boolean } | undefined {
  if (!url.pathname.startsWith(FILE_PATH)) return undefined;
  const blobId = url.pathname.slice(FILE_PATH.length).split('/')[0] ?? '';
  if (!/^blob_[0-9A-Z]{26}$/.test(blobId)) return undefined;
  return { blobId, download: url.searchParams.get('dl') === '1' };
}
