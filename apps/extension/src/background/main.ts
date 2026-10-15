// Service worker de l'extension : seul propriétaire de la connexion au relais
// (note 0009). Les écouteurs Chrome sont enregistrés tout de suite, au
// premier niveau, comme Manifest V3 l'exige ; le moteur (WebAssembly,
// secrets, connexion) s'ouvre au premier événement.

import init from '../wasm/ot_wasm.js';
import type { Item, OffscreenMessage, Push, Reply, Request, StateView } from '../shared/api';
import { POPUP_PORT, channelOf } from '../shared/api';
import { humanSize } from '../shared/format';
import * as opfs from '../shared/opfs';
import { relayOrigin } from '../shared/origins';
import { AUTO_CACHE_MAX, fileUrl, isViewable, parseFilePath } from '../shared/files';
import { Engine, type FileStorage } from './engine';
import { type KeyValue, IndexedDbKeySource } from './secrets';

const KEEPALIVE_ALARM = 'keepalive';
const MENU_ROOT = 'send';
const MENU_ALL = 'to:';
const COMMAND_SEND_PAGE = 'send-page';
const COMMAND_SEND_CLIPBOARD = 'send-clipboard';
/**
 * Un élément reçu s'ouvre de lui-même (navigateur au premier plan) seulement
 * s'il est récent : à la réouverture du navigateur, les messages en attente
 * donnent des notifications, pas une rafale d'onglets.
 */
const AUTO_OPEN_MAX_AGE = 2 * 60_000;
const NOTIFICATION_COPY = 0;
/** Délai pour que la popup ouverte pour un élément le réclame (note 0032). */
const PENDING_CHAT_MAX_AGE = 10_000;

function log(message: string, error?: unknown): void {
  // Jamais de contenu ni de clé : seulement le message de l'erreur.
  if (error === undefined) console.warn(`[oldtown] ${message}`);
  else
    console.warn(`[oldtown] ${message} :`, error instanceof Error ? error.message : String(error));
}

const storage: KeyValue = {
  get: (keys) => chrome.storage.local.get(keys),
  set: (items) => chrome.storage.local.set(items),
  remove: (keys) => chrome.storage.local.remove(keys),
};

/** Fichiers dans l'OPFS de l'extension (note 0026). */
const files: FileStorage = {
  source: (path) => opfs.source(path),
  remove: (path) => opfs.remove(path),
  getChunk: (blobId, index) => opfs.getChunk(blobId, index),
  putChunk: (blobId, index, sealed) => opfs.putChunk(blobId, index, sealed),
  removeCache: (blobId) => opfs.removeCache(blobId),
  listCache: () => opfs.listCache(),
};

let opening: Promise<Engine> | undefined;
const popups = new Set<chrome.runtime.Port>();
/**
 * Conversation à montrer dans la prochaine popup (note 0032) : posé juste
 * avant de l'ouvrir pour un élément reçu, consommé à sa connexion. En
 * mémoire seulement : la popup s'ouvre dans la foulée.
 */
let pendingChat: { channel: string; until: number } | undefined;

/** Ouvre le moteur une seule fois par vie du service worker. */
function engine(): Promise<Engine> {
  opening ??= (async () => {
    await init({ module_or_path: chrome.runtime.getURL('wasm/ot_wasm_bg.wasm') });
    const e = await Engine.open({
      kv: storage,
      keys: new IndexedDbKeySource(),
      files,
      relayAllowed: (url) => chrome.permissions.contains({ origins: [relayOrigin(url)] }),
      log,
    });
    e.onChange((push) => onPush(e, push));
    await updateMenus(e.state());
    return e;
  })().catch((err: unknown) => {
    opening = undefined;
    throw err;
  });
  return opening;
}

// Permission d'accès au relais accordée (ou retirée) : les fichiers en
// attente repartent.
chrome.permissions.onAdded.addListener(() => void engine().then((e) => e.refreshRelayAccess()));
chrome.permissions.onRemoved.addListener(() => void engine().then((e) => e.refreshRelayAccess()));

// --- Démarrage, réveil ---

/** Requête `fetch` reçue par le service worker (types absents de la lib DOM). */
interface FetchLike {
  readonly request: globalThis.Request;
  respondWith(response: Promise<Response> | Response): void;
}

/**
 * Fichiers reçus (note 0029) : le service worker répond lui-même aux adresses
 * `chrome-extension://<id>/f/<blob_id>/<nom>`, en flux, déchiffrées dans le
 * WebAssembly ; le navigateur les affiche ou les télécharge. Ces adresses ne
 * sont pas accessibles aux pages web (pas de `web_accessible_resources`).
 */
(self as unknown as EventTarget).addEventListener('fetch', (event) => {
  const fetchEvent = event as unknown as FetchLike;
  const url = new URL(fetchEvent.request.url);
  if (url.origin !== new URL(chrome.runtime.getURL('')).origin) return;
  const file = parseFilePath(url);
  if (!file) return;
  const range = fetchEvent.request.headers.get('range');
  fetchEvent.respondWith(
    engine()
      .then((e) => e.fileResponse(file.blobId, { download: file.download, range }))
      .catch((err: unknown) => {
        log('fichier reçu', err);
        return new Response('Fichier indisponible.', {
          status: 500,
          headers: { 'Content-Type': 'text/plain; charset=utf-8' },
        });
      }),
  );
});

chrome.runtime.onInstalled.addListener(() => void wake());
chrome.runtime.onStartup.addListener(() => void wake());
chrome.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name === KEEPALIVE_ALARM) void wake();
});

/**
 * Se connecte s'il le faut. L'alarme (30 s, le minimum) relance le service
 * worker s'il a été arrêté pendant une attente de reconnexion (note 0023).
 */
async function wake(): Promise<void> {
  await chrome.alarms.create(KEEPALIVE_ALARM, { periodInMinutes: 0.5 });
  try {
    (await engine()).ensureConnected();
  } catch (e) {
    log('démarrage', e);
  }
}

// --- Événements du moteur ---

function onPush(e: Engine, push: Push): void {
  for (const port of popups) port.postMessage(push);
  if (push.type === 'state') void updateMenus(push.state);
  if (push.type === 'item') void deliverItem(e, push.item);
  if (push.type === 'state') void updateBadge(e.state());
}

async function updateBadge(state: StateView): Promise<void> {
  const problem = state.phase === 'ready' && state.connection !== 'online';
  await chrome.action.setBadgeText({ text: problem ? '!' : '' });
}

// --- Popup et réglages ---

chrome.runtime.onConnect.addListener((port) => {
  if (port.name !== POPUP_PORT || !fromThisExtension(port.sender)) {
    port.disconnect();
    return;
  }
  popups.add(port);
  port.onDisconnect.addListener(() => popups.delete(port));
  // Seule la popup (ou sa fenêtre de repli) réclame la conversation, pas
  // les réglages ouverts dans un onglet.
  const chat = isPopupPage(port.sender) ? takePendingChat() : undefined;
  void engine()
    .then((e) => {
      port.postMessage({ type: 'state', state: e.state() } satisfies Push);
      if (chat !== undefined) port.postMessage({ type: 'open', channel: chat } satisfies Push);
    })
    .catch((err: unknown) => log('ouverture de la popup', err));
});

function isPopupPage(sender?: chrome.runtime.MessageSender): boolean {
  if (!sender?.url) return false;
  return new URL(sender.url).pathname === '/index.html';
}

function takePendingChat(): string | undefined {
  const chat = pendingChat;
  pendingChat = undefined;
  return chat && Date.now() < chat.until ? chat.channel : undefined;
}

/** Retient la conversation d'un élément reçu, pour la popup qui va s'ouvrir. */
function rememberChat(e: Engine, item: Item): void {
  pendingChat = {
    channel: channelOf(item, e.state().device?.id),
    until: Date.now() + PENDING_CHAT_MAX_AGE,
  };
}

/**
 * Seules les pages de cette extension (popup, réglages ouverts dans un
 * onglet) parlent au service worker : même identifiant et une URL
 * `chrome-extension://<id>/`, ce qu'aucune page web ni autre extension ne
 * peut présenter (pas d'`externally_connectable`, pas de script de contenu).
 */
function fromThisExtension(sender?: chrome.runtime.MessageSender): boolean {
  return (
    sender?.id === chrome.runtime.id && (sender.url?.startsWith(chrome.runtime.getURL('')) ?? false)
  );
}

chrome.runtime.onMessage.addListener((msg: Request & { target?: string }, sender, reply) => {
  if (msg.target === 'offscreen') return false;
  if (!fromThisExtension(sender)) return false;
  handle(msg).then(
    (value) => reply({ ok: true, value } satisfies Reply),
    (err: unknown) =>
      reply({ ok: false, error: err instanceof Error ? err.message : String(err) } satisfies Reply),
  );
  return true;
});

async function handle(req: Request): Promise<unknown> {
  const e = await engine();
  switch (req.type) {
    case 'state':
      return e.state();
    case 'setup-offer':
      return e.setupOffer(req.serverUrl, req.name);
    case 'setup-code':
      return e.setupCode(req.text, req.serverUrl, req.name);
    case 'cancel-pairing':
    case 'forget':
      return e.forget();
    case 'send':
    case 'send-clipboard':
      await e.setRecipient(req.to);
      return e.send(req.text, req.to);
    case 'set-recipient':
      return e.setRecipient(req.to);
    case 'history':
      return e.history();
    case 'clear-history':
      return e.clearHistory();
    case 'devices':
      return e.devices();
    case 'rotate':
      return e.rotate(req.exclude);
    case 'set-server-url':
      return e.setServerUrl(req.url);
    case 'send-file':
      await e.setRecipient(req.to);
      return e.sendFile({
        path: req.path,
        name: req.name,
        mime: req.mime,
        size: req.size,
        to: req.to,
      });
  }
}

// --- Menus contextuels et raccourci ---

let menuKey = '';

/** Un sous-menu par membre du roster, plus « Tous les appareils ». */
async function updateMenus(state: StateView): Promise<void> {
  const others =
    state.group?.members
      .filter((m) => m.device_id !== state.device?.id)
      .map((m) => [m.device_id, m.name]) ?? [];
  const key = JSON.stringify(others);
  if (key === menuKey) return;
  menuKey = key;
  await chrome.contextMenus.removeAll();
  if (state.phase !== 'ready') return;
  const contexts: [
    `${chrome.contextMenus.ContextType}`,
    ...`${chrome.contextMenus.ContextType}`[],
  ] = ['page', 'link', 'selection', 'image'];
  chrome.contextMenus.create({ id: MENU_ROOT, title: 'Envoyer à OldTown', contexts });
  for (const [id, name] of others) {
    chrome.contextMenus.create({ id: `to:${id}`, parentId: MENU_ROOT, title: name, contexts });
  }
  if (others.length > 1) {
    chrome.contextMenus.create({
      id: MENU_ALL,
      parentId: MENU_ROOT,
      title: 'Tous les appareils',
      contexts,
    });
  }
}

chrome.contextMenus.onClicked.addListener((info, tab) => {
  const id = String(info.menuItemId);
  if (!id.startsWith('to:')) return;
  const to = id.slice(3);
  if (info.mediaType === 'image' && info.srcUrl) {
    void sendImage(info.srcUrl, to);
    return;
  }
  // Sélection, sinon lien, sinon la page.
  const text = info.selectionText ?? info.linkUrl ?? info.pageUrl ?? tab?.url;
  if (text) void sendFromBrowser(text, to);
});

/**
 * « Envoyer l'image » (note 0027) : lue sans cookies. Si le site ne
 * l'autorise pas (CORS) et que son domaine n'a pas encore été accordé, la
 * page d'envoi demande l'accès à ce seul domaine, sur un clic.
 */
async function sendImage(src: string, to: string): Promise<void> {
  try {
    const res = await fetch(src, { credentials: 'omit' });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const blob = await res.blob();
    const path = `outgoing/${crypto.randomUUID()}`;
    await opfs.writeBlob(path, blob);
    const e = await engine();
    await e.setRecipient(to);
    await flashSent();
    await e.sendFile({
      path,
      name: imageName(src, blob.type),
      mime: blob.type,
      size: blob.size,
      to,
    });
  } catch (err) {
    log('lecture de l’image', err);
    // Le domaine n'est pas (encore) accordé : la page d'envoi le demande.
    const query = new URLSearchParams({ image: src, to });
    await chrome.windows.create({
      url: `send.html?${query.toString()}`,
      type: 'popup',
      width: 420,
      height: 360,
    });
  }
}

/** Nom de l'image d'après son URL, avec une extension d'après son type. */
function imageName(src: string, mime: string): string {
  let base = 'image';
  try {
    const last = new URL(src).pathname.split('/').pop();
    if (last) base = decodeURIComponent(last);
  } catch {
    // URL `data:` ou illisible : nom générique.
  }
  const ext = /^image\/(\w+)/.exec(mime)?.[1]?.replace('jpeg', 'jpg');
  return ext && !base.toLowerCase().endsWith(`.${ext}`) ? `${base}.${ext}` : base;
}

chrome.commands.onCommand.addListener((command, tab) => {
  if (command === COMMAND_SEND_PAGE) {
    // L'URL de l'onglet actif (permission `activeTab`) ; la sélection
    // demanderait un script injecté, exclu par le plan (note 0023).
    const url = tab?.url;
    if (url) void engine().then((e) => sendFromBrowser(url, e.state().recipient, true));
  } else if (command === COMMAND_SEND_CLIPBOARD) {
    void sendClipboard();
  }
});

/** Raccourci : le presse-papiers, lu par le document offscreen, au dernier destinataire. */
async function sendClipboard(): Promise<void> {
  let text: string;
  try {
    text = await readClipboard();
  } catch (err) {
    log('lecture du presse-papiers', err);
    text = '';
  }
  const e = await engine();
  if (!text.trim()) {
    await problem('Le presse-papiers ne contient pas de texte.');
    return;
  }
  await sendFromBrowser(text, e.state().recipient, true);
}

async function problem(message: string): Promise<void> {
  await chrome.notifications.create({
    type: 'basic',
    iconUrl: 'icons/icon-128.png',
    title: 'OldTown : envoi impossible',
    message,
  });
}

/** Coche brève sur l'icône : un raccourci n'a pas d'autre retour visible. */
async function flashSent(): Promise<void> {
  await chrome.action.setBadgeText({ text: '✓' });
  setTimeout(() => void engine().then((e) => updateBadge(e.state())), 1500);
}

async function sendFromBrowser(text: string, to: string, shortcut = false): Promise<void> {
  try {
    const e = await engine();
    await e.setRecipient(to);
    await e.send(text, to);
    if (shortcut) await flashSent();
  } catch (err) {
    log('envoi', err);
    await problem(err instanceof Error ? err.message : String(err));
  }
}

// --- Notifications ---

function isWebLink(item: Item): boolean {
  return item.kind === 'link' && /^https?:\/\//i.test(item.body);
}

/** Fenêtre du navigateur qui a le focus de l'OS, s'il y en a une. */
async function focusedWindow(): Promise<chrome.windows.Window | undefined> {
  try {
    const w = await chrome.windows.getLastFocused();
    return w.focused ? w : undefined;
  } catch {
    return undefined;
  }
}

/**
 * Ouvre la popup de l'extension (Chromium 127 ou plus, fenêtre au premier
 * plan) ; renvoie `false` si le navigateur refuse.
 */
async function openPopup(windowId?: number): Promise<boolean> {
  try {
    await chrome.action.openPopup(windowId === undefined ? {} : { windowId });
    return true;
  } catch (e) {
    log('ouverture de la popup', e);
    return false;
  }
}

/**
 * Élément reçu (note 0024) : popup ouverte, il n'apparaît que dans la liste ;
 * navigateur au premier plan, un lien s'ouvre dans un onglet et un texte
 * ouvre la popup, sans notification ; sinon, notification.
 */
async function deliver(e: Engine, item: Item): Promise<void> {
  if (popups.size > 0) return;
  const recent = Date.now() - Date.parse(item.createdAt) < AUTO_OPEN_MAX_AGE;
  const win = recent ? await focusedWindow() : undefined;
  if (win) {
    if (isWebLink(item)) {
      await chrome.tabs.create({ url: item.body, active: true, windowId: win.id });
      return;
    }
    rememberChat(e, item);
    if (await openPopup(win.id)) return;
  }
  await notify(item);
}

/**
 * Annonce d'un élément. Fichier (note 0029), à sa première annonce seulement
 * (les mises à jour, copie terminée ou erreur, ne disent rien) : popup
 * ouverte, rien ; navigateur au premier plan et élément récent, un petit
 * fichier que le navigateur sait afficher s'ouvre dans un onglet, un gros
 * ouvre la popup ; sinon, et pour un type que le navigateur ne sait pas
 * afficher, notification. Rien n'arrive sur le disque sans geste.
 */
async function deliverItem(e: Engine, item: Item): Promise<void> {
  // Un élément envoyé d'ici (note 0030) n'est annoncé nulle part.
  if (item.outgoing) return;
  const file = item.file;
  if (!file) {
    await deliver(e, item);
    return;
  }
  if (file.cached || file.error || popups.size > 0) return;
  const recent = Date.now() - Date.parse(item.createdAt) < AUTO_OPEN_MAX_AGE;
  const win = recent ? await focusedWindow() : undefined;
  if (win && e.state().relayAccess) {
    if (file.size > AUTO_CACHE_MAX) {
      rememberChat(e, item);
      if (await openPopup(win.id)) return;
    } else if (isViewable(file.mime)) {
      await chrome.tabs.create({
        url: fileUrl(file.blobId, file.name),
        active: true,
        windowId: win.id,
      });
      return;
    }
  }
  await notify(item);
}

/** Ce qu'un clic sur un fichier fait : l'ouvrir, le télécharger, ou montrer la popup. */
function fileAction(item: Item): 'open' | 'download' | 'popup' {
  const file = item.file;
  if (!file || file.size > AUTO_CACHE_MAX) return 'popup';
  return isViewable(file.mime) ? 'open' : 'download';
}

/** Notification d'un élément reçu ; l'identifiant est son `msg_id`. */
async function notify(item: Item): Promise<void> {
  const file = item.file;
  const hint = {
    open: 'Cliquer pour ouvrir',
    download: 'Cliquer pour télécharger',
    popup: 'Cliquer pour voir dans OldTown',
  };
  await chrome.notifications.create(item.msgId, {
    type: 'basic',
    iconUrl: 'icons/icon-128.png',
    title: item.fromName,
    message: file ? `📎 ${file.name} (${humanSize(file.size)})` : item.body.slice(0, 500),
    contextMessage: file
      ? hint[fileAction(item)]
      : isWebLink(item)
        ? 'Cliquer pour ouvrir'
        : undefined,
    buttons: file ? undefined : [{ title: 'Copier' }],
  });
}

/**
 * Fichier depuis le service worker (clic sur une notification) : ouvert dans
 * un onglet, ou téléchargé par un onglet en arrière-plan, que le navigateur
 * referme une fois le téléchargement lancé.
 */
async function openFileItem(e: Engine, item: Item): Promise<void> {
  const file = item.file;
  if (!file) return;
  const action = fileAction(item);
  if (action === 'popup') {
    rememberChat(e, item);
    await showExtension();
    return;
  }
  const download = action === 'download';
  await chrome.tabs.create({ url: fileUrl(file.blobId, file.name, download), active: !download });
}

chrome.notifications.onClicked.addListener((id) => {
  void (async () => {
    await chrome.notifications.clear(id);
    const e = await engine();
    const item = await e.item(id);
    // « Ouvrir » seulement pour un lien http(s), comme sur Android.
    if (item && isWebLink(item)) {
      await chrome.tabs.create({ url: item.body });
      return;
    }
    if (item?.file) {
      await openFileItem(e, item);
      return;
    }
    if (item) rememberChat(e, item);
    await showExtension();
  })().catch((e: unknown) => log('notification', e));
});

/**
 * Montre l'extension après un clic sur une notification : la popup si le
 * navigateur l'accepte, sinon la même page dans une petite fenêtre.
 */
async function showExtension(): Promise<void> {
  const win = await focusedWindow();
  if (win && (await openPopup(win.id))) return;
  await chrome.windows.create({ url: 'index.html', type: 'popup', width: 400, height: 640 });
}

chrome.notifications.onButtonClicked.addListener((id, button) => {
  if (button !== NOTIFICATION_COPY) return;
  void (async () => {
    // Le texte est relu dans l'historique, pas porté par la notification.
    const item = await (await engine()).item(id);
    if (item) await copy(item.body);
    await chrome.notifications.clear(id);
  })().catch((e: unknown) => log('copie', e));
});

// --- Presse-papiers (document offscreen) ---

let creatingOffscreen: Promise<void> | undefined;

async function offscreen<T>(message: OffscreenMessage): Promise<T> {
  const url = chrome.runtime.getURL('offscreen.html');
  const contexts = await chrome.runtime.getContexts({
    contextTypes: [chrome.runtime.ContextType.OFFSCREEN_DOCUMENT],
    documentUrls: [url],
  });
  if (contexts.length === 0) {
    creatingOffscreen ??= chrome.offscreen
      .createDocument({
        url: 'offscreen.html',
        reasons: [chrome.offscreen.Reason.CLIPBOARD],
        justification: 'Copier un élément reçu, ou lire le presse-papiers pour un raccourci',
      })
      .finally(() => (creatingOffscreen = undefined));
    await creatingOffscreen;
  }
  return (await chrome.runtime.sendMessage(message)) as T;
}

async function copy(text: string): Promise<void> {
  await offscreen<boolean>({ target: 'offscreen', type: 'copy', text });
}

/** Texte du presse-papiers (permission `clipboardRead`), au raccourci seulement. */
async function readClipboard(): Promise<string> {
  return (await offscreen<string | null>({ target: 'offscreen', type: 'read' })) ?? '';
}
