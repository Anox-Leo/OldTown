// Page d'envoi de fichiers (note 0026), ouverte dans une petite fenêtre : le
// sélecteur de fichiers de l'OS peut fermer la popup, et la copie dans l'OPFS
// doit aller à son terme. Sert aussi aux demandes de permission d'hôte
// (note 0027), qui exigent un clic : `?image=<url>&to=<id>` (image d'une page
// que le site ne laisse pas lire) et `?grant=relay` (premier accès au
// relais). Le choix de fichiers se fait d'ordinaire dans la popup (note
// 0029) ; la page garde le sien en repli. Le service worker fait le reste ; la page peut se fermer.

import type { Reply, Request, StateView } from '../shared/api';
import { humanSize } from '../shared/format';
import { writeBlob } from '../shared/opfs';
import { originPattern, relayOrigin } from '../shared/origins';

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const params = new URLSearchParams(location.search);
const image = params.get('image');
const grantRelay = params.get('grant') === 'relay';

async function request<T>(req: Request): Promise<T> {
  const reply = (await chrome.runtime.sendMessage(req)) as Reply<T> | undefined;
  if (!reply) throw new Error('le service worker ne répond pas');
  if (!reply.ok) throw new Error(reply.error);
  return reply.value;
}

function fail(e: unknown): void {
  $('error').textContent = e instanceof Error ? e.message : String(e);
  $<HTMLButtonElement>('go').disabled = false;
}

/** Demande les origines manquantes (sur un clic) ; `false` si refusé. */
async function grant(origins: string[]): Promise<boolean> {
  if (await chrome.permissions.contains({ origins })) return true;
  return chrome.permissions.request({ origins });
}

/** Copie dans l'OPFS puis confie l'envoi au service worker (sans l'attendre). */
async function send(blob: Blob, name: string, to: string): Promise<void> {
  const path = `outgoing/${crypto.randomUUID()}`;
  await writeBlob(path, blob, (done, total) => {
    $('status').textContent = `Préparation de ${name} : ${Math.floor((done * 100) / total)} %`;
  });
  const req: Request = { type: 'send-file', path, name, mime: blob.type, size: blob.size, to };
  // L'envoi continue dans le service worker même si la fenêtre se ferme.
  void request<string>(req).catch(() => undefined);
}

async function main(): Promise<void> {
  const state = await request<StateView>({ type: 'state' });
  const device = state.device;
  if (state.phase !== 'ready' || !device) {
    $('error').textContent = 'Cette extension n’est pas encore dans un groupe.';
    return;
  }
  const relay = relayOrigin(device.serverUrl);
  const select = $<HTMLSelectElement>('to');
  for (const m of state.group?.members ?? []) {
    if (m.device_id === device.id) continue;
    select.add(new Option(m.name, m.device_id));
  }
  select.value = params.get('to') ?? state.recipient;
  const go = $<HTMLButtonElement>('go');
  const files = $<HTMLInputElement>('files');

  if (grantRelay) {
    $('title').textContent = 'Accès au relais';
    $('ask').textContent =
      `Autoriser OldTown à télécharger les fichiers depuis ${new URL(relay.replace('/*', '')).hostname} ?`;
    $('ask').hidden = false;
    files.hidden = true;
    select.hidden = true;
    go.textContent = 'Autoriser';
    go.disabled = false;
    go.onclick = async () => {
      go.disabled = true;
      try {
        if (!(await grant([relay]))) throw new Error('Accès refusé.');
        // Le service worker relance seul ce qui attendait l'accès.
        window.close();
      } catch (e) {
        fail(e);
      }
    };
    return;
  }

  if (image) {
    const host = new URL(image).hostname;
    $('title').textContent = 'Envoyer l’image';
    $('ask').textContent =
      `Ce site ne laisse pas lire ses images. Autoriser OldTown à lire les images de ${host} ? (une fois pour ce domaine, révocable dans les réglages du navigateur)`;
    $('ask').hidden = false;
    files.hidden = true;
    go.textContent = 'Autoriser et envoyer';
    go.disabled = false;
    go.onclick = async () => {
      go.disabled = true;
      $('error').textContent = '';
      try {
        if (!(await grant([relay, originPattern(image)]))) throw new Error('Accès refusé.');
        const res = await fetch(image, { credentials: 'omit' });
        if (!res.ok) throw new Error(`image illisible (HTTP ${res.status})`);
        const blob = await res.blob();
        const name = decodeURIComponent(new URL(image).pathname.split('/').pop() || 'image');
        await send(blob, name, select.value);
        window.close();
      } catch (e) {
        fail(e);
      }
    };
    return;
  }

  files.onchange = () => {
    const list = [...(files.files ?? [])];
    const total = list.reduce((n, f) => n + f.size, 0);
    $('status').textContent = list.length
      ? `${list.length} fichier${list.length > 1 ? 's' : ''}, ${humanSize(total)}`
      : '';
    go.disabled = list.length === 0;
  };
  go.onclick = async () => {
    go.disabled = true;
    $('error').textContent = '';
    try {
      if (!(await grant([relay]))) throw new Error('Accès au relais refusé.');
      for (const file of files.files ?? []) await send(file, file.name, select.value);
      $('status').textContent = 'Envoi lancé : suivi dans la popup.';
      setTimeout(() => window.close(), 1200);
    } catch (e) {
      fail(e);
    }
  };
}

main().catch(fail);
