// Mises en forme partagées par la popup, les pages et le service worker.

/** Taille lisible : `1,5 Mo`. */
export function humanSize(bytes: number): string {
  const units = ['o', 'Ko', 'Mo', 'Go'];
  let v = bytes;
  let u = 0;
  while (v >= 1024 && u < units.length - 1) {
    v /= 1024;
    u++;
  }
  return u === 0 ? `${bytes} o` : `${v.toFixed(1).replace('.', ',')} ${units[u]}`;
}
