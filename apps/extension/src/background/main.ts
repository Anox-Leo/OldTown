// Service worker de l'extension : seul propriétaire de la connexion au relais
// (note 0009). Les écouteurs Chrome sont enregistrés tout de suite, au
// premier niveau, comme Manifest V3 l'exige ; le moteur (WebAssembly,
// secrets, connexion) s'ouvre au premier événement.

import init from '../wasm/ot_wasm.js';
import type { Item, OffscreenMessage, Push, Reply, Request, StateView } from '../shared/api';
import { POPUP_PORT } from '../shared/api';
import { Engine } from './engine';
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

let opening: Promise<Engine> | undefined;
const popups = new Set<chrome.runtime.Port>();

/** Ouvre le moteur une seule fois par vie du service worker. */
function engine(): Promise<Engine> {
  opening ??= (async () => {
    await init({ module_or_path: chrome.runtime.getURL('wasm/ot_wasm_bg.wasm') });
    const e = await Engine.open({ kv: storage, keys: new IndexedDbKeySource(), log });
    e.onChange((push) => onPush(e, push));
    await updateMenus(e.state());
    return e;
  })().catch((err: unknown) => {
    opening = undefined;
    throw err;
  });
  return opening;
}

// --- Démarrage, réveil ---

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
  if (push.type === 'item') void deliver(push.item);
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
  void engine()
    .then((e) => port.postMessage({ type: 'state', state: e.state() } satisfies Push))
    .catch((err: unknown) => log('ouverture de la popup', err));
});

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
  ] = ['page', 'link', 'selection'];
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
  // Sélection, sinon lien, sinon la page.
  const text = info.selectionText ?? info.linkUrl ?? info.pageUrl ?? tab?.url;
  if (text) void sendFromBrowser(text, to);
});

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
async function deliver(item: Item): Promise<void> {
  if (popups.size > 0) return;
  const recent = Date.now() - Date.parse(item.createdAt) < AUTO_OPEN_MAX_AGE;
  const win = recent ? await focusedWindow() : undefined;
  if (win) {
    if (isWebLink(item)) {
      await chrome.tabs.create({ url: item.body, active: true, windowId: win.id });
      return;
    }
    if (await openPopup(win.id)) return;
  }
  await notify(item);
}

/** Notification d'un élément reçu ; l'identifiant est son `msg_id`. */
async function notify(item: Item): Promise<void> {
  await chrome.notifications.create(item.msgId, {
    type: 'basic',
    iconUrl: 'icons/icon-128.png',
    title: item.fromName,
    message: item.body.slice(0, 500),
    contextMessage: isWebLink(item) ? 'Cliquer pour ouvrir' : undefined,
    buttons: [{ title: 'Copier' }],
  });
}

chrome.notifications.onClicked.addListener((id) => {
  void (async () => {
    await chrome.notifications.clear(id);
    const item = await (await engine()).item(id);
    // « Ouvrir » seulement pour un lien http(s), comme sur Android.
    if (item && isWebLink(item)) {
      await chrome.tabs.create({ url: item.body });
      return;
    }
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
