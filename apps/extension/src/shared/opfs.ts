// Fichiers de l'extension dans l'OPFS (Origin Private File System, notes
// 0026 et 0029) : copies des fichiers à envoyer (`outgoing/<id>`, en clair,
// effacées après l'envoi) et morceaux **chiffrés** des fichiers reçus
// (`cache/<blob_id>/<n>`), déchiffrés seulement quand le service worker les
// sert. Partagé par le service worker et les pages, de même origine.

import type { ByteSource } from '../background/transfer';

async function root(): Promise<FileSystemDirectoryHandle> {
  return navigator.storage.getDirectory();
}

/** Dossier parent d'un chemin `a/b/c` (créé au besoin) et nom final. */
async function parent(path: string, create: boolean): Promise<[FileSystemDirectoryHandle, string]> {
  const parts = path.split('/').filter(Boolean);
  const name = parts.pop();
  if (!name) throw new Error(`chemin invalide : ${path}`);
  let dir = await root();
  for (const p of parts) dir = await dir.getDirectoryHandle(p, { create });
  return [dir, name];
}

export async function fileHandle(path: string, create = false): Promise<FileSystemFileHandle> {
  const [dir, name] = await parent(path, create);
  return dir.getFileHandle(name, { create });
}

/** Le fichier (lecture seule, sans tout charger en mémoire). */
export async function readFile(path: string): Promise<File> {
  return (await fileHandle(path)).getFile();
}

/** Copie `blob` sous `path`, morceau par morceau (un `File` choisi par l'utilisateur). */
export async function writeBlob(
  path: string,
  blob: Blob,
  onProgress?: (done: number, total: number) => void,
): Promise<void> {
  const writable = await (await fileHandle(path, true)).createWritable();
  try {
    const step = 4 << 20;
    for (let offset = 0; offset < blob.size; offset += step) {
      await writable.write(await blob.slice(offset, offset + step).arrayBuffer());
      onProgress?.(Math.min(offset + step, blob.size), blob.size);
    }
    await writable.close();
  } catch (e) {
    await writable.abort().catch(() => undefined);
    throw e;
  }
}

/** Efface un fichier ou un dossier ; sans effet s'il n'existe pas. */
export async function remove(path: string): Promise<void> {
  try {
    const [dir, name] = await parent(path, false);
    await dir.removeEntry(name, { recursive: true });
  } catch (e) {
    if (!(e instanceof DOMException && e.name === 'NotFoundError')) throw e;
  }
}

/** Noms des entrées d'un dossier (vide s'il n'existe pas). */
export async function list(path: string): Promise<string[]> {
  try {
    const parts = path.split('/').filter(Boolean);
    let dir = await root();
    for (const p of parts) dir = await dir.getDirectoryHandle(p);
    const names: string[] = [];
    for await (const name of (dir as unknown as { keys(): AsyncIterable<string> }).keys()) {
      names.push(name);
    }
    return names;
  } catch {
    return [];
  }
}

export async function source(path: string): Promise<ByteSource> {
  const file = await readFile(path);
  return {
    size: file.size,
    read: async (offset, length) =>
      new Uint8Array(await file.slice(offset, offset + length).arrayBuffer()),
  };
}

/** Morceau chiffré `index` de la copie d'un fichier reçu, s'il y est. */
export async function getChunk(blobId: string, index: number): Promise<Uint8Array | undefined> {
  try {
    return new Uint8Array(await (await readFile(`cache/${blobId}/${index}`)).arrayBuffer());
  } catch (e) {
    if (
      e instanceof DOMException &&
      (e.name === 'NotFoundError' || e.name === 'TypeMismatchError')
    ) {
      return undefined;
    }
    throw e;
  }
}

/**
 * Range un morceau chiffré. Écrit sous un nom temporaire puis déplacé : un
 * morceau est entier ou absent, même si le service worker s'arrête en plein
 * milieu.
 */
export async function putChunk(blobId: string, index: number, sealed: Uint8Array): Promise<void> {
  const tmp = `cache/${blobId}/${index}.tmp`;
  await writeBlob(tmp, new Blob([sealed as BlobPart]));
  const handle = (await fileHandle(tmp)) as FileSystemFileHandle & {
    move(name: string): Promise<void>;
  };
  await handle.move(String(index));
}

/** Efface la copie d'un fichier reçu. */
export function removeCache(blobId: string): Promise<void> {
  return remove(`cache/${blobId}`);
}

/** Fichiers reçus dont une copie existe. */
export function listCache(): Promise<string[]> {
  return list('cache');
}
