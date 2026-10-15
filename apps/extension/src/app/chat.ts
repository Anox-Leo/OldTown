import {
  ChangeDetectionStrategy,
  Component,
  ElementRef,
  afterRenderEffect,
  computed,
  inject,
  input,
  output,
  signal,
  viewChild,
} from '@angular/core';
import { FormsModule } from '@angular/forms';

import {
  type FileView,
  type Item,
  type StateView,
  type TransferView,
  channelOf,
} from '../shared/api';
import { filePath, isViewable } from '../shared/files';
import { humanSize } from '../shared/format';
import { writeBlob } from '../shared/opfs';
import { AvatarComponent } from './avatar';
import { BackgroundService } from './background.service';
import { ALL_NAME, avatarColor, deviceKind } from './devices';
import { IconComponent } from './icon';
import { connectionStatus, shortTime } from './status';

/**
 * Conversation d'un canal (notes 0030 et 0031) : « Tous » ou un appareil du
 * groupe. Les messages envoyés d'ici sont à droite, les reçus à gauche ; la
 * zone d'envoi, en bas, envoie dans ce canal. La flèche revient à la liste.
 */
@Component({
  selector: 'app-chat',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AvatarComponent, FormsModule, IconComponent],
  template: `
    @let s = state();
    <header class="row">
      <button class="icon ghost" (click)="back.emit()" title="Retour" aria-label="Retour">
        <app-icon name="arrow-left" />
      </button>
      <app-avatar [kind]="kind()" [color]="color()" [size]="36" />
      <div class="grow title">
        <strong class="name">{{ channelName() }}</strong>
        @if (subtitle(); as sub) {
          <span class="small presence" [class.ok]="sub.online">{{ sub.text }}</span>
        }
      </div>
    </header>
    @if (status().help; as help) {
      <div class="banner warning">{{ help }}</div>
    }
    @if (!s.relayAccess) {
      <div class="banner warning row">
        <span class="grow">Fichiers : autoriser l'accès au relais.</span>
        <button class="small" (click)="grantRelay()">Autoriser</button>
      </div>
    }

    <ol class="messages" #list aria-live="polite">
      @for (item of visible(); track item.msgId) {
        <li class="msg" [class.out]="item.outgoing" [class.in]="!item.outgoing">
          @if (!item.outgoing && channel() === '') {
            <span class="from small">{{ item.fromName }}</span>
          }
          <div class="line">
            <div
              class="bubble"
              [class.clickable]="clickable(item)"
              [attr.role]="clickable(item) ? 'button' : null"
              [attr.tabindex]="clickable(item) ? 0 : null"
              [title]="hint(item)"
              (click)="activate(item)"
              (keydown.enter)="activate(item)"
            >
              @if (item.file; as f) {
                <span class="file">
                  <app-icon name="paperclip" />
                  <span class="name">{{ f.name }}</span>
                </span>
                <span class="meta small">{{ size(f.size) }}</span>
                @if (progress(item); as p) {
                  <progress class="thin" [value]="p.done" [max]="p.total || 1"></progress>
                }
                @if (f.error; as e) {
                  <span class="error small">{{ e }}</span>
                }
              } @else {
                <span
                  class="text"
                  [class.link]="isLink(item)"
                  [class.expanded]="expanded() === item.msgId"
                  >{{ item.body }}</span
                >
              }
            </div>
            @if (item.file; as f) {
              @if (!item.outgoing) {
                <a
                  class="icon side"
                  [href]="downloadPath(f)"
                  target="_blank"
                  rel="noopener"
                  (click)="onDownload($event, f)"
                  title="Télécharger"
                  aria-label="Télécharger"
                >
                  <app-icon name="download" />
                </a>
              }
            } @else {
              <button
                class="icon side"
                (click)="copy(item)"
                [title]="copied() === item.msgId ? 'Copié' : 'Copier'"
                aria-label="Copier"
              >
                <app-icon [name]="copied() === item.msgId ? 'check' : 'copy'" />
              </button>
            }
          </div>
          <span class="time small">
            {{ time(item.createdAt) }}{{ item.pending ? ' · envoi…' : '' }}
          </span>
        </li>
      } @empty {
        <li class="empty muted">
          {{
            channel() === ''
              ? 'Aucun message pour tous vos appareils.'
              : 'Aucun message avec ' + channelName() + '.'
          }}
        </li>
      }
      @for (t of uploads(); track t.id) {
        <li class="msg out">
          <div class="line">
            <div class="bubble">
              <span class="file">
                <app-icon name="paperclip" />
                <span class="name">{{ t.name }}</span>
              </span>
              <span class="meta small">{{ percent(t) }} %</span>
              <progress class="thin" [value]="t.done" [max]="t.total || 1"></progress>
              @if (t.error; as e) {
                <span class="error small">{{ e }}</span>
              }
            </div>
          </div>
        </li>
      }
    </ol>

    <footer class="composer">
      @if (bg.error(); as error) {
        <div class="banner danger" role="alert">{{ error }}</div>
      } @else if (notice(); as n) {
        <p class="small muted notice">{{ n }}</p>
      }
      <div class="row">
        <input
          #picker
          type="file"
          multiple
          hidden
          aria-hidden="true"
          (change)="sendPicked(picker)"
        />
        <button
          class="icon"
          (click)="openSendFile(picker)"
          [disabled]="busy()"
          title="Envoyer un fichier"
          aria-label="Envoyer un fichier"
        >
          <app-icon name="paperclip" />
        </button>
        <textarea
          class="grow"
          rows="1"
          [(ngModel)]="text"
          [placeholder]="placeholder()"
          aria-label="Message"
          (keydown)="onKey($event)"
        ></textarea>
        <button
          class="icon"
          (click)="sendClipboard()"
          [disabled]="busy()"
          title="Coller et envoyer le presse-papiers"
          aria-label="Coller et envoyer le presse-papiers"
        >
          <app-icon name="clipboard" />
        </button>
        <button
          class="primary icon"
          (click)="send()"
          [disabled]="busy() || !text.trim()"
          title="Envoyer (Entrée)"
          aria-label="Envoyer"
        >
          <app-icon name="send" />
        </button>
      </div>
    </footer>
  `,
  styles: `
    :host {
      display: flex;
      flex-direction: column;
      height: 580px;
      padding: 10px 12px 12px;
      gap: 8px;
    }
    header {
      flex: none;
      gap: 6px;
      margin: 0 0 0 -6px;
    }
    .icon {
      display: inline-flex;
      align-items: center;
      justify-content: center;
      padding: 7px 9px;
    }
    .ghost {
      border: none;
      background: none;
      color: var(--muted);
      padding: 6px;
    }
    .ghost:hover {
      color: var(--fg);
    }
    .title {
      display: flex;
      flex-direction: column;
      line-height: 1.25;
    }
    .title .name {
      font-weight: 600;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }
    .presence {
      color: var(--muted);
    }
    .messages {
      flex: 1;
      min-height: 0;
      overflow-y: auto;
      list-style: none;
      margin: 0;
      padding: 4px 2px;
      display: flex;
      flex-direction: column;
      gap: 8px;
    }
    .msg {
      display: flex;
      flex-direction: column;
      max-width: 86%;
    }
    .msg.in {
      align-self: flex-start;
      align-items: flex-start;
    }
    .msg.out {
      align-self: flex-end;
      align-items: flex-end;
    }
    .from {
      color: var(--muted);
      margin: 0 4px 2px;
    }
    .line {
      display: flex;
      align-items: center;
      gap: 2px;
      max-width: 100%;
    }
    .msg.out .line {
      flex-direction: row-reverse;
    }
    .bubble {
      min-width: 0;
      padding: 7px 11px;
      border-radius: 14px;
      background: var(--grouped);
      border: 1px solid var(--border);
      display: flex;
      flex-direction: column;
      gap: 3px;
    }
    .msg.in .bubble {
      border-bottom-left-radius: 4px;
    }
    .msg.out .bubble {
      background: var(--primary);
      border-color: var(--primary);
      color: var(--on-primary);
      border-bottom-right-radius: 4px;
    }
    .bubble.clickable {
      cursor: pointer;
    }
    .text {
      white-space: pre-wrap;
      overflow-wrap: anywhere;
      max-height: 9em;
      overflow: hidden;
    }
    .text.expanded {
      max-height: none;
    }
    .text.link {
      text-decoration: underline;
    }
    .msg.in .text.link {
      color: var(--primary-text);
    }
    .file {
      display: inline-flex;
      align-items: center;
      gap: 6px;
      min-width: 0;
    }
    .file .name {
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }
    .meta {
      opacity: 0.75;
    }
    .error {
      color: var(--danger-fg);
    }
    .msg.out .error {
      color: inherit;
      font-weight: 600;
    }
    progress.thin {
      width: 100%;
      height: 3px;
    }
    .side {
      flex: none;
      border: none;
      background: none;
      color: var(--muted);
      text-decoration: none;
      padding: 4px;
      opacity: 0;
    }
    .msg:hover .side,
    .side:focus-visible,
    a.side {
      opacity: 1;
    }
    .side:hover {
      color: var(--fg);
    }
    .time {
      color: var(--muted);
      margin: 2px 4px 0;
      font-size: 11px;
    }
    .empty {
      margin: auto;
      text-align: center;
      padding: 24px 12px;
    }
    .composer {
      flex: none;
      display: grid;
      gap: 6px;
    }
    .composer .row {
      align-items: flex-end;
      gap: 4px;
    }
    /* Trombone, presse-papiers et envoi : même forme, seule la couleur
       distingue l'envoi. */
    .composer .icon {
      flex: none;
      width: 38px;
      height: 38px;
      padding: 0;
    }
    .composer textarea {
      min-height: 38px;
      max-height: 120px;
      resize: none;
      field-sizing: content;
    }
    .notice {
      margin: 0;
    }
    .ok {
      color: var(--success);
    }
    .warning {
      color: var(--warning-fg);
    }
    .danger {
      color: var(--danger-fg);
    }
  `,
})
export class ChatComponent {
  protected readonly bg = inject(BackgroundService);
  protected readonly state = computed(() => this.bg.state() as StateView);
  protected readonly status = computed(() => connectionStatus(this.state()));
  /** Canal ouvert : `''` pour « Tous », sinon l'identifiant d'un appareil. */
  readonly channel = input.required<string>();
  /** Appareils en ligne d'après le relais ; `undefined` : inconnu (hors ligne ici). */
  readonly presence = input<ReadonlyMap<string, boolean>>();
  readonly back = output();

  protected readonly others = computed(
    () => this.state().group?.members.filter((m) => m.device_id !== this.state().device?.id) ?? [],
  );
  protected readonly member = computed(() =>
    this.others().find((m) => m.device_id === this.channel()),
  );
  protected readonly channelName = computed(() => this.member()?.name ?? ALL_NAME);
  protected readonly kind = computed(() => {
    const m = this.member();
    return m ? deviceKind(m.name) : 'group';
  });
  protected readonly color = computed(() => avatarColor(this.channel()));
  /** Sous le nom : « en ligne » ou « hors ligne » ; pour « Tous », combien sont en ligne. */
  protected readonly subtitle = computed(() => {
    const presence = this.presence();
    if (!presence) return undefined;
    const m = this.member();
    if (m) {
      const online = presence.get(m.device_id) ?? false;
      return { online, text: online ? 'en ligne' : 'hors ligne' };
    }
    const n = this.others().filter((o) => presence.get(o.device_id)).length;
    return { online: n > 0, text: n ? `${n} en ligne` : 'aucun appareil en ligne' };
  });
  /** Messages du canal, du plus ancien au plus récent. */
  protected readonly visible = computed(() => {
    const me = this.state().device?.id;
    return this.bg
      .items()
      .filter((i) => channelOf(i, me) === this.channel())
      .sort((a, b) => Date.parse(a.createdAt) - Date.parse(b.createdAt));
  });
  /** Fichiers en cours d'envoi, en bas de la conversation. */
  protected readonly uploads = computed(() =>
    this.state().transfers.filter((t) => t.direction === 'up'),
  );
  protected readonly placeholder = computed(() =>
    this.channel() === '' ? 'Message à tous vos appareils' : `Message à ${this.channelName()}`,
  );
  protected readonly busy = signal(false);
  protected readonly notice = signal<string | undefined>(undefined);
  protected readonly copied = signal<string | undefined>(undefined);
  /** Texte déplié dans la conversation. */
  protected readonly expanded = signal<string | undefined>(undefined);
  protected text = '';

  private readonly list = viewChild<ElementRef<HTMLElement>>('list');

  constructor() {
    // Le dernier message en vue, à l'ouverture, au changement de canal et à
    // chaque nouveau message.
    afterRenderEffect(() => {
      this.visible();
      this.uploads();
      const el = this.list()?.nativeElement;
      if (el) el.scrollTop = el.scrollHeight;
    });
  }

  protected time(iso: string): string {
    return shortTime(iso);
  }

  protected size(bytes: number): string {
    return humanSize(bytes);
  }

  protected percent(t: TransferView): number {
    return t.total ? Math.floor((t.done * 100) / t.total) : 100;
  }

  /** Copie en cours d'un fichier reçu, pour la barre fine de la bulle. */
  protected progress(item: Item): TransferView | undefined {
    return this.state().transfers.find((t) => t.id === item.msgId && !t.error);
  }

  /** Lien de téléchargement d'un fichier reçu, servi par le service worker (note 0029). */
  protected downloadPath(f: FileView): string {
    return filePath(f.blobId, f.name, true).slice(1);
  }

  /**
   * Icône de téléchargement : téléchargement natif du navigateur, toujours
   * disponible (l'annuler ne change rien à l'élément). C'est une navigation
   * dans un nouvel onglet, que le service worker sert en `attachment` et que
   * le navigateur referme : un lien `download` irait chercher l'adresse sans
   * passer par le service worker, et échouerait. Sans accès au relais et
   * sans copie, la fenêtre de permission d'abord.
   */
  protected onDownload(event: Event, f: FileView): void {
    if (this.state().relayAccess || f.cached) return;
    event.preventDefault();
    void this.grantRelay();
  }

  /** Un fichier envoyé d'ici n'a pas de copie à ouvrir ; tout le reste s'ouvre. */
  protected clickable(item: Item): boolean {
    return !(item.file && item.outgoing);
  }

  protected hint(item: Item): string {
    const f = item.file;
    if (f) return item.outgoing ? '' : isViewable(f.mime) ? 'Ouvrir' : 'Télécharger';
    return this.isLink(item) ? 'Ouvrir le lien' : 'Afficher en entier';
  }

  /**
   * Clic sur une bulle : un fichier reçu que le navigateur sait afficher
   * s'ouvre dans un onglet, un autre se télécharge ; un lien s'ouvre ; un
   * texte se déplie.
   */
  protected async activate(item: Item): Promise<void> {
    const f = item.file;
    if (!f) {
      if (this.isLink(item)) await chrome.tabs.create({ url: item.body });
      else this.expanded.update((id) => (id === item.msgId ? undefined : item.msgId));
      return;
    }
    if (item.outgoing) return;
    if (!this.state().relayAccess && !f.cached) {
      await this.grantRelay();
      return;
    }
    if (isViewable(f.mime)) {
      await chrome.tabs.create({ url: chrome.runtime.getURL(filePath(f.blobId, f.name).slice(1)) });
      return;
    }
    // Navigation (voir `onDownload`), pas un lien `download`.
    const a = document.createElement('a');
    a.href = this.downloadPath(f);
    a.target = '_blank';
    a.rel = 'noopener';
    a.click();
  }

  /**
   * Trombone (note 0029) : le sélecteur de fichiers de l'OS, directement ;
   * le fichier part dans le canal ouvert. Sans accès au relais, la fenêtre
   * de permission d'abord (une demande depuis la popup peut la fermer).
   */
  protected async openSendFile(picker: HTMLInputElement): Promise<void> {
    if (!this.state().relayAccess) {
      await this.grantRelay();
      return;
    }
    picker.value = '';
    picker.click();
  }

  /**
   * Copie les fichiers choisis dans l'OPFS (le service worker ne reçoit pas
   * de `File` par message), puis confie l'envoi au service worker sans
   * l'attendre : il continue si la popup se ferme.
   */
  protected async sendPicked(picker: HTMLInputElement): Promise<void> {
    const files = [...(picker.files ?? [])];
    if (files.length === 0) return;
    const to = this.channel();
    this.bg.error.set(undefined);
    this.busy.set(true);
    try {
      for (const file of files) {
        const path = `outgoing/${crypto.randomUUID()}`;
        await writeBlob(path, file, (done, total) => {
          this.notice.set(`Préparation de ${file.name} : ${Math.floor((done * 100) / total)} %`);
        });
        const req = {
          type: 'send-file',
          path,
          name: file.name,
          mime: file.type,
          size: file.size,
          to,
        } as const;
        void this.bg.request<string>(req).catch((e: unknown) => {
          this.bg.error.set(e instanceof Error ? e.message : String(e));
        });
      }
      this.notice.set(undefined);
    } catch (e) {
      this.bg.error.set(`Fichier non préparé : ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      this.busy.set(false);
      picker.value = '';
    }
  }

  /** Fenêtre de permission (un clic y est nécessaire ; la popup se fermerait). */
  protected async grantRelay(): Promise<void> {
    await chrome.windows.create({
      url: 'send.html?grant=relay',
      type: 'popup',
      width: 420,
      height: 240,
    });
  }

  protected isLink(item: Item): boolean {
    return item.kind === 'link' && /^https?:\/\//i.test(item.body);
  }

  /** Entrée envoie, Maj+Entrée va à la ligne (et rien pendant une composition IME). */
  protected onKey(event: KeyboardEvent): void {
    if (event.key === 'Enter' && !event.shiftKey && !event.isComposing) {
      event.preventDefault();
      void this.send();
    }
  }

  protected async send(): Promise<void> {
    if (!this.text.trim()) return;
    this.busy.set(true);
    const sent = await this.bg.run<string>({ type: 'send', text: this.text, to: this.channel() });
    this.busy.set(false);
    if (sent) {
      this.text = '';
      this.notice.set(this.offlineNotice());
    }
  }

  protected async sendClipboard(): Promise<void> {
    this.bg.error.set(undefined);
    let text: string;
    try {
      // Lecture seulement à ce clic (permission clipboardRead, note 0023).
      text = await navigator.clipboard.readText();
    } catch {
      this.bg.error.set('Presse-papiers illisible.');
      return;
    }
    if (!text.trim()) {
      this.bg.error.set('Le presse-papiers ne contient pas de texte.');
      return;
    }
    this.busy.set(true);
    const sent = await this.bg.run<string>({ type: 'send-clipboard', text, to: this.channel() });
    this.busy.set(false);
    if (sent) this.notice.set(this.offlineNotice());
  }

  /** Hors ligne, un mot pour dire que l'envoi partira au retour de la connexion. */
  private offlineNotice(): string | undefined {
    return this.state().connection === 'online'
      ? undefined
      : 'Hors ligne : envoi au retour de la connexion.';
  }

  protected async copy(item: Item): Promise<void> {
    try {
      await navigator.clipboard.writeText(item.body);
      this.copied.set(item.msgId);
    } catch {
      this.bg.error.set('Copie impossible.');
    }
  }
}
