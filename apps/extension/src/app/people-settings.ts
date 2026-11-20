import { NgTemplateOutlet } from '@angular/common';
import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';

import type { PersonDeviceView, StateView } from '../shared/api';
import { BackgroundService } from './background.service';
import { deviceKind } from './devices';

type Kind = NonNullable<PersonDeviceView['kind']>;

const KINDS: readonly { value: Kind; label: string }[] = [
  { value: 'phone', label: 'Téléphone' },
  { value: 'laptop', label: 'Portable' },
  { value: 'desktop', label: 'Ordinateur fixe' },
  { value: 'browser', label: 'Navigateur' },
  { value: 'cli', label: 'Ligne de commande' },
];

/**
 * Sections des réglages de l'étape 10 : annuaire à publier (admin), alertes
 * de l'annuaire, mes appareils (A2 : par défaut, joignable, catégorie,
 * retirer ; A4 : « Ligne de commande » repliée), services (couper), lien vers
 * l'administration.
 */
@Component({
  selector: 'app-people-settings',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [NgTemplateOutlet],
  template: `
    @let s = state();
    @if (s.rootMissing; as root) {
      <section class="card">
        <h2>Annuaire</h2>
        <p class="small">
          Cet appareil est à l'admin, mais l'annuaire n'est pas encore publié. Sa racine portera le
          nom <strong>{{ root.name }}</strong>, signé une fois pour toutes : s'il ne convient pas,
          le changer d'abord sur le relais (<code>ot-server user rename owner &lt;nom&gt;</code>).
        </p>
        <button class="primary" (click)="adminInit()" [disabled]="busy()">
          Publier l'annuaire
        </button>
      </section>
    }

    @if (s.alerts.length) {
      <section class="card">
        <h2>Alertes de l'annuaire</h2>
        @for (a of s.alerts; track $index) {
          <p class="small danger">
            @switch (a.kind) {
              @case ('fork') {
                Le relais publie pour vous un autre groupe que le vôtre : n'ajoutez ni ne retirez
                d'appareil avant d'avoir vérifié.
              }
              @case ('rejected') {
                Objet refusé pour {{ nameOf($any(a).user) }} : {{ $any(a).reason }}.
              }
              @case ('keys_changed') {
                Les clés de {{ nameOf($any(a).user) }} ont changé (nouvel appareil après une
                réinvitation ?).
              }
            }
          </p>
        }
      </section>
    }

    @if (me(); as person) {
      <section class="card">
        <h2>Mes appareils</h2>
        <p class="small muted">
          Une autre personne qui vous écrit vise vos appareils par défaut ; elle peut choisir un de
          vos appareils joignables.
        </p>
        @for (d of usual(); track d.device_id) {
          <ng-container *ngTemplateOutlet="device; context: { $implicit: d }" />
        }
        @if (cli().length) {
          <details>
            <summary class="small">Ligne de commande ({{ cli().length }})</summary>
            <p class="small muted">
              Membres du groupe (« Tous mes appareils » les inclut), jamais joignables ni par
              défaut.
            </p>
            @for (d of cli(); track d.device_id) {
              <ng-container *ngTemplateOutlet="device; context: { $implicit: d }" />
            }
          </details>
        }
      </section>

      <ng-template #device let-d>
        <div class="device">
          <div class="row">
            <strong class="grow">{{ d.name }}</strong>
            <span class="small" [class.ok]="d.online" [class.muted]="!d.online">
              {{ d.online ? 'en ligne' : 'hors ligne' }}
            </span>
          </div>
          <div class="row small settings">
            <select
              (change)="setKind(d, $any($event.target).value)"
              [attr.aria-label]="'Catégorie de ' + d.name"
            >
              @for (k of kinds; track k.value) {
                <option [value]="k.value" [selected]="k.value === kindOf(d)">{{ k.label }}</option>
              }
            </select>
            @if (kindOf(d) !== 'cli') {
              <label class="toggle">
                <input
                  type="checkbox"
                  [checked]="d.default"
                  (change)="update(d, { default: $any($event.target).checked })"
                />
                par défaut
              </label>
              <label class="toggle">
                <input
                  type="checkbox"
                  [checked]="d.reachable"
                  (change)="update(d, { reachable: $any($event.target).checked })"
                />
                joignable
              </label>
            }
            <span class="grow"></span>
            @if (d.device_id === s.device?.id) {
              <span class="muted">cet appareil</span>
            } @else if (confirm() === d.device_id) {
              <button class="danger" (click)="remove(d)" [disabled]="busy()">Confirmer</button>
              <button (click)="confirm.set(undefined)">Annuler</button>
            } @else {
              <button (click)="confirm.set(d.device_id)">Retirer…</button>
            }
          </div>
          @if (!d.up_to_date) {
            <p class="small warning">Pas à jour : il ne recevra rien des autres personnes.</p>
          }
          @if (confirm() === d.device_id) {
            <p class="small muted">
              Nouvelle clé de groupe sans lui, puis révocation par le relais : il ne lira plus rien.
            </p>
          }
        </div>
      </ng-template>
    }

    @if (services().length) {
      <section class="card">
        <h2>Services</h2>
        @for (svc of services(); track svc.user) {
          <label class="row toggle">
            <span class="grow">{{ svc.name }}</span>
            <input
              type="checkbox"
              [checked]="!svc.muted"
              (change)="mute(svc.user, !$any($event.target).checked)"
            />
            reçu
          </label>
        }
        <p class="small muted">Un service coupé ne vous envoie plus rien.</p>
      </section>
    }

    @if (s.admin) {
      <section class="card">
        <h2>Administration</h2>
        <p class="small muted">Personnes, invitations et réseau, dans une page entière.</p>
        <button (click)="openAdmin()">Ouvrir l'administration</button>
      </section>
    }

    @if (error(); as e) {
      <div class="banner danger" role="alert">{{ e }}</div>
    }
  `,
  styles: `
    h2 {
      font-size: 15px;
      margin: 0 0 8px;
    }
    section.card {
      padding: 14px 16px;
      margin-bottom: 12px;
    }
    .device {
      padding: 8px 0;
      border-top: 1px solid var(--border);
    }
    .settings {
      flex-wrap: wrap;
      gap: 10px;
      margin-top: 4px;
    }
    select {
      width: auto;
    }
    .toggle {
      display: inline-flex;
      align-items: center;
      gap: 4px;
      margin: 0;
    }
    .toggle input {
      width: auto;
    }
    details summary {
      cursor: pointer;
      padding: 8px 0;
      border-top: 1px solid var(--border);
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
export class PeopleSettingsComponent {
  protected readonly bg = inject(BackgroundService);
  protected readonly state = computed(() => this.bg.state() as StateView);
  protected readonly me = computed(() =>
    this.state().people.find((p) => p.user === this.state().me),
  );
  protected readonly usual = computed(() =>
    (this.me()?.devices ?? []).filter((d) => this.kindOf(d) !== 'cli'),
  );
  protected readonly cli = computed(() =>
    (this.me()?.devices ?? []).filter((d) => this.kindOf(d) === 'cli'),
  );
  protected readonly services = computed(() =>
    this.state().people.filter((p) => p.role === 'service'),
  );
  protected readonly kinds = KINDS;
  protected readonly busy = signal(false);
  protected readonly confirm = signal<string | undefined>(undefined);
  protected readonly error = signal<string | undefined>(undefined);

  protected kindOf(d: PersonDeviceView): Kind {
    return deviceKind(d.name, d.kind);
  }

  protected nameOf(user: string): string {
    return this.state().people.find((p) => p.user === user)?.name ?? user;
  }

  private async act(req: Parameters<BackgroundService['request']>[0]): Promise<void> {
    this.busy.set(true);
    this.error.set(undefined);
    try {
      await this.bg.request(req);
    } catch (e) {
      this.error.set(e instanceof Error ? e.message : String(e));
    } finally {
      this.busy.set(false);
    }
  }

  protected adminInit(): Promise<void> {
    return this.act({ type: 'admin-init' });
  }

  protected setKind(d: PersonDeviceView, kind: Kind): Promise<void> {
    // Ligne de commande : jamais joignable ni par défaut (le relais le refuse).
    const settings = kind === 'cli' ? { kind, reachable: false, default: false } : { kind };
    return this.update(d, settings);
  }

  protected update(
    d: PersonDeviceView,
    settings: { kind?: Kind; reachable?: boolean; default?: boolean },
  ): Promise<void> {
    return this.act({ type: 'update-device', device: d.device_id, settings });
  }

  protected async remove(d: PersonDeviceView): Promise<void> {
    await this.act({ type: 'remove-device', device: d.device_id });
    this.confirm.set(undefined);
  }

  protected mute(service: string, muted: boolean): Promise<void> {
    return this.act({ type: 'mute-service', service, muted });
  }

  protected openAdmin(): Promise<chrome.tabs.Tab> {
    return chrome.tabs.create({ url: 'admin.html' });
  }
}
