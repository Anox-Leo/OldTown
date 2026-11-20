import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  signal,
  untracked,
} from '@angular/core';
import { FormsModule } from '@angular/forms';

import type { InvitationView, StateView } from '../shared/api';
import type { AdminUser } from '../background/frames';
import { BackgroundService } from './background.service';
import { LogoComponent } from './brand';
import { QrComponent } from './qr';
import { shortTime } from './status';

export type Link = 'both' | 'none' | 'one-way';

export function linkBetween(users: readonly AdminUser[], a: string, b: string): Link {
  const ab = users.find((u) => u.id === a)?.links.includes(b) ?? false;
  const ba = users.find((u) => u.id === b)?.links.includes(a) ?? false;
  return ab && ba ? 'both' : ab || ba ? 'one-way' : 'none';
}

export function gridPairs<T>(rows: readonly T[]): [T, T][] {
  const pairs: [T, T][] = [];
  rows.forEach((a, i) => rows.slice(i + 1).forEach((b) => pairs.push([a, b])));
  return pairs;
}

@Component({
  selector: 'app-admin',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [FormsModule, LogoComponent, QrComponent],
  template: `
    <h1><app-logo [size]="32" /> <span class="muted">Administration</span></h1>
    @if (!state().admin) {
      <p class="muted">Réservé à l'appareil de l'admin, une fois l'annuaire publié.</p>
    } @else {
      <section class="card">
        <h2>Inviter une personne</h2>
        @if (invitation(); as inv) {
          @if (joined(); as j) {
            <p class="ok">
              <strong>{{ j.name }}</strong> a rejoint depuis {{ j.deviceName }}. Empreinte :
              <span class="mono">{{ j.fingerprint }}</span>
            </p>
            <button (click)="invitation.set(undefined)">Inviter quelqu'un d'autre</button>
          } @else {
            <p class="small">
              Pour <strong>{{ inv.name }}</strong>, valable jusqu'au {{ day(inv.expiresAt) }}. Le
              lien et le code contiennent un secret : ne les donnez qu'à elle (SMS, en main propre).
            </p>
            <div class="invite">
              <div class="qr"><app-qr [qr]="inv.qr" label="QR d'invitation" /></div>
              <div class="grow">
                <label for="link">Lien</label>
                <div class="row">
                  <input id="link" class="grow mono" [value]="inv.link" readonly />
                  <button (click)="copy(inv.link)">Copier</button>
                </div>
                <label for="code"
                  >Code seul (CLI :
                  <code>ot invite join &lt;code&gt; --server &lt;url&gt;</code>)</label
                >
                <div class="row">
                  <input id="code" class="grow mono" [value]="inv.code" readonly />
                  <button (click)="copy(inv.code)">Copier</button>
                </div>
                <p class="small muted">
                  En attente de sa demande. Cette page peut se fermer : l'extension la certifiera à
                  son arrivée.
                </p>
              </div>
            </div>
            <button (click)="invitation.set(undefined)">Fermer</button>
          }
        } @else {
          <label for="name">Nom</label>
          <input id="name" [(ngModel)]="name" maxlength="64" autocomplete="off" />
          @if (people().length) {
            <p class="small muted">Liée à :</p>
            <div class="links">
              @for (p of people(); track p.id) {
                <label class="toggle">
                  <input
                    type="checkbox"
                    [checked]="links().has(p.id)"
                    (change)="toggleLink(p.id)"
                  />
                  {{ p.name }}
                </label>
              }
            </div>
          }
          <button class="primary" [disabled]="busy() || !name.trim()" (click)="invite()">
            Créer l'invitation
          </button>
        }
      </section>

      <section class="card">
        <h2>Personnes</h2>
        @for (u of persons(); track u.id) {
          <div class="person">
            <div class="row">
              @if (renaming() === u.id) {
                <input class="grow" [(ngModel)]="newName" maxlength="64" />
                <button class="primary" (click)="rename(u)" [disabled]="busy()">Renommer</button>
                <button (click)="renaming.set(undefined)">Annuler</button>
              } @else {
                <strong class="grow">{{ u.name }}</strong>
                <span class="small muted">{{ roleLabel(u) }}</span>
              }
            </div>
            <ul class="devices small">
              @for (d of u.devices; track d.id) {
                <li>
                  <span [class.ok]="d.online">●</span> {{ d.name }}
                  <span class="muted">
                    {{ d.default ? '· par défaut' : '' }}{{ d.reachable ? '' : ' · injoignable' }} ·
                    {{ d.online ? 'en ligne' : lastSeen(d.last_seen_at) }}
                  </span>
                </li>
              } @empty {
                <li class="muted">Aucun appareil actif.</li>
              }
            </ul>
            @if (u.role !== 'admin' && renaming() !== u.id) {
              <div class="row small">
                <button (click)="startRename(u)">Renommer…</button>
                @if (u.role === 'member') {
                  <button (click)="reinvite(u)" [disabled]="busy()">Réinviter</button>
                }
                @if (revoking() === u.id) {
                  <button class="danger" (click)="revoke(u)" [disabled]="busy()">Confirmer</button>
                  <button (click)="revoking.set(undefined)">Annuler</button>
                } @else {
                  <button (click)="revoking.set(u.id)">Révoquer…</button>
                }
              </div>
              @if (revoking() === u.id) {
                <p class="small muted">
                  Tous ses appareils révoqués, ses liens retirés, ses messages en attente effacés.
                </p>
              }
            }
          </div>
        } @empty {
          <p class="small muted">Chargement…</p>
        }
      </section>

      @if (pairs().length) {
        <section class="card">
          <h2>Réseau</h2>
          <p class="small muted">Qui peut écrire à qui : les liens vont dans les deux sens.</p>
          <ul class="grid">
            @for (pair of pairs(); track pair[0].id + pair[1].id) {
              @let link = linkOf(pair[0].id, pair[1].id);
              <li>
                <label class="toggle">
                  <input
                    type="checkbox"
                    [checked]="link === 'both'"
                    [indeterminate]="link === 'one-way'"
                    (change)="setPair(pair[0].id, pair[1].id, $any($event.target).checked)"
                    [disabled]="busy()"
                  />
                  {{ pair[0].name }} ↔ {{ pair[1].name }}
                </label>
                @if (link === 'one-way') {
                  <span class="small warning">à sens unique</span>
                }
              </li>
            }
          </ul>
        </section>
      }

      @if (services().length) {
        <section class="card">
          <h2>Services</h2>
          <p class="small muted">
            Un service écrit aux personnes cochées ; elles ne lui répondent pas, et peuvent le
            couper.
          </p>
          @for (svc of services(); track svc.id) {
            <div class="person">
              <strong>{{ svc.name }}</strong>
              <div class="links">
                @for (p of people(); track p.id) {
                  <label class="toggle small">
                    <input
                      type="checkbox"
                      [checked]="svc.links.includes(p.id)"
                      (change)="setLink(svc.id, p.id, $any($event.target).checked)"
                      [disabled]="busy()"
                    />
                    {{ p.name }}{{ p.muted?.includes(svc.id) ? ' (coupé)' : '' }}
                  </label>
                }
              </div>
            </div>
          }
        </section>
      }
    }
    @if (error(); as e) {
      <div class="banner danger" role="alert">{{ e }}</div>
    }
  `,
  styles: `
    :host {
      display: block;
      max-width: 760px;
      margin: 24px auto;
      padding: 0 16px;
    }
    h1 {
      display: flex;
      align-items: center;
      gap: 12px;
      margin: 0 0 20px;
      font-weight: 600;
    }
    h2 {
      font-size: 15px;
      margin: 0 0 8px;
    }
    section.card {
      padding: 14px 16px;
      margin-bottom: 12px;
    }
    .invite {
      display: flex;
      gap: 16px;
      align-items: flex-start;
      margin: 8px 0;
    }
    .qr {
      flex: none;
      width: 180px;
    }
    .links {
      display: flex;
      flex-wrap: wrap;
      gap: 6px 16px;
      margin: 4px 0 10px;
    }
    .toggle {
      display: inline-flex;
      align-items: center;
      gap: 6px;
      margin: 0;
    }
    .toggle input {
      width: auto;
    }
    .person {
      padding: 8px 0;
      border-top: 1px solid var(--border);
    }
    .devices {
      list-style: none;
      margin: 4px 0;
      padding: 0;
    }
    .grid {
      list-style: none;
      margin: 0;
      padding: 0;
      display: grid;
      grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
      gap: 6px 12px;
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
export class AdminComponent {
  protected readonly bg = inject(BackgroundService);
  protected readonly state = computed(() => this.bg.state() as StateView);
  protected readonly users = signal<AdminUser[]>([]);
  protected readonly invitation = signal<InvitationView | undefined>(undefined);
  protected readonly links = signal(new Set<string>());
  protected readonly busy = signal(false);
  protected readonly error = signal<string | undefined>(undefined);
  protected readonly renaming = signal<string | undefined>(undefined);
  protected readonly revoking = signal<string | undefined>(undefined);
  protected name = '';
  protected newName = '';

  protected readonly persons = computed(() =>
    this.users()
      .filter((u) => u.role !== 'service' && u.state !== 'revoked')
      .sort((a, b) => Number(b.role === 'admin') - Number(a.role === 'admin')),
  );
  protected readonly people = computed(() => this.persons().filter((u) => u.state === 'active'));
  protected readonly services = computed(() =>
    this.users().filter((u) => u.role === 'service' && u.state !== 'revoked'),
  );
  protected readonly pairs = computed(() => gridPairs(this.people()));
  protected readonly joined = computed(() => {
    const j = this.bg.joined();
    return j && j.user === this.invitation()?.user ? j : undefined;
  });

  private linksTouched = false;

  constructor() {
    void this.load();
    effect(() => {
      const me = this.bg.state()?.me;
      if (me && !untracked(() => this.linksTouched)) this.links.set(new Set([me]));
    });
  }

  protected async load(): Promise<void> {
    try {
      this.users.set(await this.bg.request<AdminUser[]>({ type: 'users' }));
    } catch (e) {
      this.error.set(e instanceof Error ? e.message : String(e));
    }
  }

  private async act<T>(req: Parameters<BackgroundService['request']>[0]): Promise<T | undefined> {
    this.busy.set(true);
    this.error.set(undefined);
    try {
      const value = await this.bg.request<T>(req);
      await this.load();
      return value;
    } catch (e) {
      this.error.set(e instanceof Error ? e.message : String(e));
      return undefined;
    } finally {
      this.busy.set(false);
    }
  }

  protected linkOf(a: string, b: string): Link {
    return linkBetween(this.users(), a, b);
  }

  protected roleLabel(u: AdminUser): string {
    if (u.role === 'admin') return 'admin';
    return u.state === 'pending' ? 'invitée, pas encore là' : 'membre';
  }

  protected lastSeen(ms?: number): string {
    return ms ? `vu ${shortTime(new Date(ms).toISOString())}` : 'jamais vu';
  }

  protected day(iso: string): string {
    return new Date(iso).toLocaleString('fr-FR', {
      day: 'numeric',
      month: 'long',
      hour: '2-digit',
      minute: '2-digit',
    });
  }

  protected toggleLink(id: string): void {
    this.linksTouched = true;
    this.links.update((set) => {
      const next = new Set(set);
      if (!next.delete(id)) next.add(id);
      return next;
    });
  }

  protected async invite(): Promise<void> {
    const inv = await this.act<InvitationView>({
      type: 'invite',
      name: this.name,
      links: [...this.links()],
    });
    if (inv) {
      this.invitation.set(inv);
      this.name = '';
      this.bg.joined.set(undefined);
    }
  }

  protected async reinvite(u: AdminUser): Promise<void> {
    const inv = await this.act<InvitationView>({ type: 'reinvite', user: u.id });
    if (inv) this.invitation.set(inv);
  }

  protected startRename(u: AdminUser): void {
    this.newName = u.name;
    this.renaming.set(u.id);
  }

  protected async rename(u: AdminUser): Promise<void> {
    await this.act({ type: 'rename-user', user: u.id, name: this.newName });
    this.renaming.set(undefined);
  }

  protected async revoke(u: AdminUser): Promise<void> {
    await this.act({ type: 'revoke-user', user: u.id });
    this.revoking.set(undefined);
  }

  protected async setPair(a: string, b: string, allowed: boolean): Promise<void> {
    await this.act({ type: 'link-set', from: a, to: b, allowed });
    await this.act({ type: 'link-set', from: b, to: a, allowed });
  }

  protected setLink(from: string, to: string, allowed: boolean): Promise<unknown> {
    return this.act({ type: 'link-set', from, to, allowed });
  }

  protected async copy(text: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      this.error.set('Copie impossible.');
    }
  }
}
