import { describe, expect, it } from 'vitest';

import { filePath, parseFilePath, saveName } from '../src/shared/files';

const BLOB = 'blob_01J9Z3R7XK2M4N6P8Q0R2S4T6V';
const at = (path: string) => new URL(`chrome-extension://abc${path}`);

describe('adresse d’un fichier reçu (B2 : « Enregistrer sous »)', () => {
  it('rend le blob et le nom à proposer', () => {
    expect(parseFilePath(at(filePath(BLOB, 'photo été.jpg')))).toEqual({
      blobId: BLOB,
      name: 'photo été.jpg',
      download: false,
    });
    expect(parseFilePath(at(filePath(BLOB, 'a.pdf', true)))?.download).toBe(true);
  });

  it('ignore ce qui n’est pas un fichier reçu', () => {
    expect(parseFilePath(at('/index.html'))).toBeUndefined();
    expect(parseFilePath(at('/f/pas_un_blob/a.txt'))).toBeUndefined();
  });

  it('un nom sûr pour le disque', () => {
    expect(saveName('../../.bashrc')).toBe('_.._.bashrc');
    expect(saveName('a/b\\c:d*?.txt')).toBe('a_b_c_d__.txt');
    expect(saveName('...')).toBe('fichier');
    expect(parseFilePath(at(`/f/${BLOB}/`))?.name).toBe(BLOB);
    expect(parseFilePath(at(`/f/${BLOB}/%E0%A4%A`))?.name).toBe(BLOB);
  });
});
