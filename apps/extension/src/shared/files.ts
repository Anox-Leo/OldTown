export const FILE_PATH = '/f/';

export const AUTO_CACHE_MAX = 20 * 1024 * 1024;

const VIEWABLE =
  /^(image\/(png|jpeg|gif|webp|avif|bmp)|application\/pdf|text\/plain|audio\/(mpeg|mp4|ogg|wav|webm|aac|flac)|video\/(mp4|webm|ogg|quicktime))$/;

export function isViewable(mime: string): boolean {
  return VIEWABLE.test(mime.toLowerCase());
}

export function filePath(blobId: string, name: string, download = false): string {
  return `${FILE_PATH}${blobId}/${encodeURIComponent(name)}${download ? '?dl=1' : ''}`;
}

export function fileUrl(blobId: string, name: string, download = false): string {
  return chrome.runtime.getURL(filePath(blobId, name, download).slice(1));
}

export function parseFilePath(
  url: URL,
): { blobId: string; name: string; download: boolean } | undefined {
  if (!url.pathname.startsWith(FILE_PATH)) return undefined;
  const [blobId = '', name = ''] = url.pathname.slice(FILE_PATH.length).split('/');
  if (!/^blob_[0-9A-Z]{26}$/.test(blobId)) return undefined;
  let decoded: string;
  try {
    decoded = decodeURIComponent(name);
  } catch {
    decoded = '';
  }
  return {
    blobId,
    name: saveName(decoded || blobId),
    download: url.searchParams.get('dl') === '1',
  };
}

export function saveName(name: string): string {
  return name.replace(/[\\/:*?"<>|\p{Cc}]/gu, '_').replace(/^[.\s]+/, '') || 'fichier';
}
