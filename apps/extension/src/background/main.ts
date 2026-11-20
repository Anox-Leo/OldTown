import init from '../wasm/ot_wasm.js';
import type { Item, OffscreenMessage, Push, Reply, Request, StateView } from '../shared/api';
import { POPUP_PORT, channelOf, userChannel } from '../shared/api';
import { humanSize } from '../shared/format';
import * as opfs from '../shared/opfs';
import { OTP_ORIGINS, relayOrigin } from '../shared/origins';
import { type FillResult, autoFillAllowed, fillOtp, isLive, sameSite } from '../shared/otp';
import { AUTO_CACHE_MAX, fileUrl, isViewable, parseFilePath } from '../shared/files';
import { Engine, type FileStorage } from './engine';
import { type KeyValue, IndexedDbKeySource } from './secrets';
import { BADGE } from '../shared/tokens';

const KEEPALIVE_ALARM = 'keepalive';
const MENU_ROOT = 'send';
const MENU_ALL = 'to:';
const COMMAND_SEND_PAGE = 'send-page';
const COMMAND_SEND_CLIPBOARD = 'send-clipboard';
const COMMAND_FILL_OTP = 'fill-otp';
const AUTO_OPEN_MAX_AGE = 2 * 60_000;
const NOTIFICATION_COPY = 0;
const NOTIFICATION_GRANT_OTP = 1;
const PENDING_CHAT_MAX_AGE = 10_000;

function log(message: string, error?: unknown): void {
  if (error === undefined) console.warn(`[oldtown] ${message}`);
  else
    console.warn(`[oldtown] ${message} :`, error instanceof Error ? error.message : String(error));
}

const storage: KeyValue = {
  get: (keys) => chrome.storage.local.get(keys),
  set: (items) => chrome.storage.local.set(items),
  remove: (keys) => chrome.storage.local.remove(keys),
};

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
let pendingChat: { channel: string; until: number } | undefined;

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

chrome.permissions.onAdded.addListener(() => void engine().then((e) => e.refreshRelayAccess()));
chrome.permissions.onRemoved.addListener(() => void engine().then((e) => e.refreshRelayAccess()));

interface FetchLike {
  readonly request: globalThis.Request;
  respondWith(response: Promise<Response> | Response): void;
}

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

async function wake(): Promise<void> {
  await chrome.alarms.create(KEEPALIVE_ALARM, { periodInMinutes: 0.5 });
  try {
    (await engine()).ensureConnected();
  } catch (e) {
    log('démarrage', e);
  }
}

function onPush(e: Engine, push: Push): void {
  for (const port of popups) port.postMessage(push);
  if (push.type === 'state') void updateMenus(push.state);
  if (push.type === 'item') void deliverItem(e, push.item);
  if (push.type === 'state') void updateBadge(e.state());
  if (push.type === 'person-joined') void personJoined(push);
}

async function personJoined(p: Extract<Push, { type: 'person-joined' }>): Promise<void> {
  if (popups.size > 0) return;
  await chrome.notifications.create(`joined:${p.user}`, {
    type: 'basic',
    iconUrl: 'icons/icon-128.png',
    title: `${p.name} a rejoint OldTown`,
    message: `Depuis ${p.deviceName}. Empreinte : ${p.fingerprint}`,
  });
}

async function updateBadge(state: StateView): Promise<void> {
  const problem = state.phase === 'ready' && state.connection !== 'online';
  if (problem) await setBadgeColors(BADGE.problem);
  await chrome.action.setBadgeText({ text: problem ? '!' : '' });
}

async function setBadgeColors(background: string): Promise<void> {
  await chrome.action.setBadgeBackgroundColor({ color: background });
  await chrome.action.setBadgeTextColor({ color: BADGE.text });
}

chrome.runtime.onConnect.addListener((port) => {
  if (port.name !== POPUP_PORT || !fromThisExtension(port.sender)) {
    port.disconnect();
    return;
  }
  popups.add(port);
  port.onDisconnect.addListener(() => popups.delete(port));
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

function rememberChat(e: Engine, item: Item): void {
  pendingChat = {
    channel: channelOf(item, e.state().device?.id),
    until: Date.now() + PENDING_CHAT_MAX_AGE,
  };
}

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
    case 'setup-invite':
      return e.setupInvite(req.text, req.name);
    case 'mark-read':
      return e.markRead(req.channel);
    case 'dismiss-lost':
      return e.dismissLostItems();
    case 'refresh-directory':
      return e.refreshDirectory();
    case 'admin-init':
      return e.adminInit();
    case 'users':
      return e.users();
    case 'invite':
      return e.invite(req.name, req.links);
    case 'reinvite':
      return e.reinvite(req.user);
    case 'revoke-user':
      return e.revokeUser(req.user);
    case 'rename-user':
      return e.renameUser(req.user, req.name);
    case 'link-set':
      return e.linkSet(req.from, req.to, req.allowed);
    case 'update-device':
      return e.updateDevice(req.device, req.settings);
    case 'remove-device':
      return e.removeDevice(req.device);
    case 'mute-service':
      return e.muteService(req.service, req.muted);
  }
}

let menuKey = '';

async function updateMenus(state: StateView): Promise<void> {
  const others =
    state.group?.members
      .filter((m) => m.device_id !== state.device?.id)
      .map((m) => [m.device_id, m.name]) ?? [];
  const people = state.people
    .filter((p) => p.user !== state.me && p.role !== 'service')
    .map((p) => [userChannel(p.user), p.name]);
  const key = JSON.stringify([others, people]);
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
  if (people.length > 0) {
    chrome.contextMenus.create({
      id: 'people',
      parentId: MENU_ROOT,
      type: 'separator',
      contexts,
    });
    for (const [channel, name] of people) {
      chrome.contextMenus.create({
        id: `to:${channel}`,
        parentId: MENU_ROOT,
        title: name,
        contexts,
      });
    }
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
  const text = info.selectionText ?? info.linkUrl ?? info.pageUrl ?? tab?.url;
  if (text) void sendFromBrowser(text, to);
});

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
    const query = new URLSearchParams({ image: src, to });
    await chrome.windows.create({
      url: `send.html?${query.toString()}`,
      type: 'popup',
      width: 420,
      height: 360,
    });
  }
}

function lastPathSegment(src: string): string | undefined {
  try {
    return decodeURIComponent(new URL(src).pathname.split('/').pop() ?? '') || undefined;
  } catch {
    return undefined;
  }
}

function imageName(src: string, mime: string): string {
  const base = lastPathSegment(src) ?? 'image';
  const ext = /^image\/(\w+)/.exec(mime)?.[1]?.replace('jpeg', 'jpg');
  return ext && !base.toLowerCase().endsWith(`.${ext}`) ? `${base}.${ext}` : base;
}

chrome.commands.onCommand.addListener((command, tab) => {
  if (command === COMMAND_SEND_PAGE) {
    const url = tab?.url;
    if (url) void engine().then((e) => sendFromBrowser(url, e.state().recipient, true));
  } else if (command === COMMAND_SEND_CLIPBOARD) {
    void sendClipboard();
  } else if (command === COMMAND_FILL_OTP && tab?.id !== undefined) {
    void fillLastOtp(tab.id);
  }
});

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

async function problem(message: string, title = 'OldTown : envoi impossible'): Promise<void> {
  await chrome.notifications.create({
    type: 'basic',
    iconUrl: 'icons/icon-128.png',
    title,
    message,
  });
}

async function flashSent(): Promise<void> {
  await setBadgeColors(BADGE.ok);
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

function isWebLink(item: Item): boolean {
  return item.kind === 'link' && /^https?:\/\//i.test(item.body);
}

async function focusedWindow(): Promise<chrome.windows.Window | undefined> {
  try {
    const w = await chrome.windows.getLastFocused();
    return w.focused ? w : undefined;
  } catch {
    return undefined;
  }
}

async function openPopup(windowId?: number): Promise<boolean> {
  try {
    await chrome.action.openPopup(windowId === undefined ? {} : { windowId });
    return true;
  } catch (e) {
    log('ouverture de la popup', e);
    return false;
  }
}

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
  await notify(e, item);
}

async function deliverItem(e: Engine, item: Item): Promise<void> {
  if (item.outgoing) return;
  if (item.kind === 'otp') {
    await deliverOtp(e, item);
    return;
  }
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
  await notify(e, item);
}

async function deliverOtp(e: Engine, item: Item): Promise<void> {
  const otp = item.otp;
  if (!otp || !isLive(otp.expiresAt)) return;
  if (!autoFillAllowed(item)) {
    if (popups.size > 0) return;
    await chrome.notifications.create(item.msgId, {
      type: 'basic',
      iconUrl: 'icons/icon-128.png',
      title: senderTitle(e, item),
      message: `Code ${otp.display} (${otp.senderName || otp.senderDomain}) : ni copié ni rempli d'office, il vient d'une autre personne.`,
      buttons: [{ title: 'Copier' }],
    });
    return;
  }
  try {
    await copy(item.body);
  } catch (err) {
    log('copie du code', err);
  }
  let filled = false;
  let grant = false;
  if (otp.verified) {
    const tab = await activeTab();
    const url = tab?.url ? new URL(tab.url) : undefined;
    if (
      tab?.id !== undefined &&
      url?.protocol === 'https:' &&
      sameSite(url.hostname, otp.senderDomain)
    ) {
      filled = (await fillTab(tab.id, item.body, false)) === 'filled';
    }
    grant = !filled && !(await chrome.permissions.contains({ origins: OTP_ORIGINS }));
  }
  if (popups.size > 0) return;
  const from = otp.senderName ? `${otp.senderName} (${otp.senderDomain})` : otp.senderDomain;
  await chrome.notifications.create(item.msgId, {
    type: 'basic',
    iconUrl: 'icons/icon-128.png',
    title: `Code ${otp.display}`,
    message: filled
      ? `${from} : rempli dans la page et copié.`
      : `${from} : copié. Le raccourci (Alt+Shift+O) le remplit dans la page.`,
    contextMessage: otp.verified
      ? undefined
      : 'Expéditeur non vérifié : regardez bien la page avant de saisir le code.',
    buttons: grant
      ? [{ title: 'Copier' }, { title: 'Remplir les codes d’office' }]
      : [{ title: 'Copier' }],
  });
}

async function activeTab(): Promise<chrome.tabs.Tab | undefined> {
  try {
    const [tab] = await chrome.tabs.query({ active: true, lastFocusedWindow: true });
    return tab;
  } catch {
    return undefined;
  }
}

async function fillTab(
  tabId: number,
  code: string,
  focused: boolean,
): Promise<FillResult | 'error'> {
  try {
    const [result] = await chrome.scripting.executeScript({
      target: { tabId },
      func: fillOtp,
      args: [code, focused],
    });
    return result?.result ?? 'error';
  } catch (err) {
    log('remplissage du code', err);
    return 'error';
  }
}

async function fillLastOtp(tabId: number): Promise<void> {
  const items = await (await engine()).history();
  const last = items
    .filter((i) => i.kind === 'otp' && !i.outgoing && i.otp && isLive(i.otp.expiresAt))
    .sort((a, b) => Date.parse(b.createdAt) - Date.parse(a.createdAt))[0];
  if (!last) {
    await problem('Aucun code reçu encore valable.', 'OldTown : pas de code');
    return;
  }
  const result = await fillTab(tabId, last.body, true);
  if (result === 'filled') await flashSent();
  else if (result === 'no-field') {
    await problem(
      'Aucun champ de code trouvé : cliquez dans le champ, puis refaites le raccourci.',
      'OldTown : code non rempli',
    );
  } else await problem('Cette page ne peut pas être remplie.', 'OldTown : code non rempli');
}

function fileAction(item: Item): 'open' | 'download' | 'popup' {
  const file = item.file;
  if (!file || file.size > AUTO_CACHE_MAX) return 'popup';
  return isViewable(file.mime) ? 'open' : 'download';
}

function senderTitle(e: Engine, item: Item): string {
  const person = item.fromUser ? e.person(item.fromUser)?.name : undefined;
  return person ? `${person} · ${item.fromName}` : item.fromName;
}

async function notify(e: Engine, item: Item): Promise<void> {
  const file = item.file;
  const hint = {
    open: 'Cliquer pour ouvrir',
    download: 'Cliquer pour télécharger',
    popup: 'Cliquer pour voir dans OldTown',
  };
  await chrome.notifications.create(item.msgId, {
    type: 'basic',
    iconUrl: 'icons/icon-128.png',
    title: senderTitle(e, item),
    message: file ? `📎 ${file.name} (${humanSize(file.size)})` : item.body.slice(0, 500),
    contextMessage: file
      ? hint[fileAction(item)]
      : isWebLink(item)
        ? 'Cliquer pour ouvrir'
        : undefined,
    buttons: file ? undefined : [{ title: 'Copier' }],
  });
}

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

async function showExtension(): Promise<void> {
  const win = await focusedWindow();
  if (win && (await openPopup(win.id))) return;
  await chrome.windows.create({ url: 'index.html', type: 'popup', width: 400, height: 640 });
}

chrome.notifications.onButtonClicked.addListener((id, button) => {
  if (button === NOTIFICATION_GRANT_OTP) {
    void grantOtp(id).catch((e: unknown) => log('accès aux sites', e));
    return;
  }
  if (button !== NOTIFICATION_COPY) return;
  void (async () => {
    const item = await (await engine()).item(id);
    if (item) await copy(item.body);
    await chrome.notifications.clear(id);
  })().catch((e: unknown) => log('copie', e));
});

async function grantOtp(id: string): Promise<void> {
  await chrome.notifications.clear(id);
  await chrome.windows.create({
    url: 'send.html?grant=otp',
    type: 'popup',
    width: 420,
    height: 300,
  });
}

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

async function readClipboard(): Promise<string> {
  return (await offscreen<string | null>({ target: 'offscreen', type: 'read' })) ?? '';
}
