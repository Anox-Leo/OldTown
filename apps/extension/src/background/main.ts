// Service worker de l'extension : seul propriétaire de la connexion au relais
// (note 0009). Les écouteurs Chrome sont enregistrés tout de suite, au
// premier niveau, comme Manifest V3 l'exige ; le moteur (WebAssembly,
// secrets, connexion) s'ouvre au premier événement.

import init from '../wasm/ot_wasm.js';
import type { Item, OffscreenCopy, Push, Reply, Request, StateView } from '../shared/api';
import { POPUP_PORT } from '../shared/api';
import { Engine } from './engine';
import { type KeyValue, IndexedDbKeySource } from './secrets';

const KEEPALIVE_ALARM = 'keepalive';
const MENU_ROOT = 'send';
const MENU_ALL = 'to:';
const COMMAND_SEND_PAGE = 'send-page';
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
  if (push.type === 'item' && popups.size === 0) void notify(push.item);
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
  if (command !== COMMAND_SEND_PAGE) return;
  // L'URL de l'onglet actif (permission `activeTab`) ; la sélection
  // demanderait un script injecté, exclu par le plan (note 0023).
  const url = tab?.url;
  if (!url) return;
  void engine().then((e) => sendFromBrowser(url, e.state().recipient));
});

async function sendFromBrowser(text: string, to: string): Promise<void> {
  try {
    const e = await engine();
    await e.setRecipient(to);
    await e.send(text, to);
  } catch (err) {
    log('envoi', err);
    await chrome.notifications.create({
      type: 'basic',
      iconUrl: 'icons/icon-128.png',
      title: 'OldTown : envoi impossible',
      message: err instanceof Error ? err.message : String(err),
    });
  }
}

// --- Notifications ---

function isWebLink(item: Item): boolean {
  return item.kind === 'link' && /^https?:\/\//i.test(item.body);
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
    const item = await (await engine()).item(id);
    // « Ouvrir » seulement pour un lien http(s), comme sur Android.
    if (item && isWebLink(item)) await chrome.tabs.create({ url: item.body });
    await chrome.notifications.clear(id);
  })().catch((e: unknown) => log('notification', e));
});

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

async function copy(text: string): Promise<void> {
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
        justification: 'Copier un élément reçu dans le presse-papiers',
      })
      .finally(() => (creatingOffscreen = undefined));
    await creatingOffscreen;
  }
  await chrome.runtime.sendMessage({
    target: 'offscreen',
    type: 'copy',
    text,
  } satisfies OffscreenCopy);
}
