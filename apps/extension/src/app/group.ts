import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';

import type { MemberView, StateView } from '../shared/api';
import { BackgroundService } from './background.service';

/** Section « Groupe » des réglages : membres et rotation de la clé. */
@Component({
  selector: 'app-group',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (state().group; as g) {
      <ul>
        @for (m of g.members; track m.device_id) {
          <li class="card">
            <div class="row">
              @if (rotating() && !isMe(m)) {
                <input
                  type="checkbox"
                  class="check"
                  [checked]="excluded().has(m.device_id)"
                  (change)="toggle(m)"
                  [attr.aria-label]="'Exclure ' + m.name"
                />
              }
              <strong class="grow">{{ m.name }}</strong>
              @if (isMe(m)) {
                <span class="small muted">cet appareil</span>
              }
            </div>
          </li>
        }
      </ul>
      @if (!rotating()) {
        <button (click)="rotating.set(true)">Nouvelle clé de groupe…</button>
        <p class="small muted">
          Pour exclure un appareil perdu ou revendu : il ne lira plus les messages suivants.
        </p>
      } @else {
        <p class="small muted">
          Cochez les appareils à exclure, puis confirmez. Les autres reçoivent la nouvelle clé.
        </p>
        <div class="row">
          <button class="primary" (click)="rotate()" [disabled]="busy()">
            {{ excluded().size ? 'Exclure et changer la clé' : 'Changer la clé' }}
          </button>
          <button (click)="cancel()">Annuler</button>
        </div>
      }
      @if (done(); as d) {
        <p class="small muted">{{ d }}</p>
      }
      @if (bg.error(); as error) {
        <div class="banner danger" role="alert">{{ error }}</div>
      }
    }
  `,
  styles: `
    ul {
      list-style: none;
      margin: 0 0 10px;
      padding: 0;
      display: grid;
      gap: 6px;
    }
    li.card {
      padding: 8px 10px;
    }
    .check {
      width: auto;
    }
  `,
})
export class GroupComponent {
  protected readonly bg = inject(BackgroundService);
  protected readonly state = computed(() => this.bg.state() as StateView);
  protected readonly rotating = signal(false);
  protected readonly excluded = signal(new Set<string>());
  protected readonly busy = signal(false);
  protected readonly done = signal<string | undefined>(undefined);

  protected isMe(m: MemberView): boolean {
    return m.device_id === this.state().device?.id;
  }

  protected toggle(m: MemberView): void {
    this.excluded.update((set) => {
      const next = new Set(set);
      if (!next.delete(m.device_id)) next.add(m.device_id);
      return next;
    });
  }

  protected cancel(): void {
    this.rotating.set(false);
    this.excluded.set(new Set());
  }

  protected async rotate(): Promise<void> {
    this.busy.set(true);
    const epoch = await this.bg.run<number>({ type: 'rotate', exclude: [...this.excluded()] });
    this.busy.set(false);
    if (epoch !== undefined) {
      this.done.set('Nouvelle clé envoyée aux autres appareils.');
      this.cancel();
    }
  }
}
