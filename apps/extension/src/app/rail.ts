import { ChangeDetectionStrategy, Component, computed, input, output } from '@angular/core';

import type { Item, PersonView } from '../shared/api';
import { AvatarComponent } from './avatar';
import { avatarColor, initials, railPeople, unread } from './devices';
import { IconComponent } from './icon';

@Component({
  selector: 'app-rail',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AvatarComponent, IconComponent],
  template: `
    <nav aria-label="Personnes">
      @if (mePerson(); as me) {
        <button
          class="person"
          [class.selected]="selected() === undefined"
          [attr.aria-current]="selected() === undefined ? 'page' : null"
          (click)="choose.emit(undefined)"
          [title]="'Mes appareils (' + me.name + ')'"
          aria-label="Mes appareils"
        >
          <app-avatar
            kind="person"
            [color]="color(me.user)"
            [initials]="initialsOf(me.name)"
            [size]="40"
          />
        </button>
      }
      <hr />
      @for (p of others(); track p.user) {
        <button
          class="person"
          [class.selected]="selected() === p.user"
          [attr.aria-current]="selected() === p.user ? 'page' : null"
          (click)="choose.emit(p.user)"
          [title]="p.name + (p.role === 'service' ? ' (service)' : '')"
          [attr.aria-label]="p.name + unreadLabel(p.user)"
        >
          <app-avatar
            kind="person"
            [color]="color(p.user)"
            [initials]="initialsOf(p.name)"
            [size]="40"
          />
          @if (count(p.user); as n) {
            <span class="badge">{{ n > 9 ? '9+' : n }}</span>
          }
          @if (online(p)) {
            <span class="dot" title="En ligne"></span>
          }
        </button>
      }
      @if (admin()) {
        <button
          class="add"
          (click)="invite.emit()"
          title="Inviter une personne"
          aria-label="Inviter une personne"
        >
          <app-icon name="plus" />
        </button>
      }
    </nav>
  `,
  styles: `
    :host {
      flex: none;
      width: 56px;
      border-right: 1px solid var(--border);
      background: var(--grouped);
      overflow-y: auto;
    }
    nav {
      display: flex;
      flex-direction: column;
      align-items: center;
      gap: 8px;
      padding: 10px 0;
    }
    hr {
      width: 28px;
      margin: 0;
      border: none;
      border-top: 2px solid var(--border);
    }
    .person,
    .add {
      position: relative;
      display: inline-flex;
      padding: 0;
      border: 2px solid transparent;
      border-radius: 50%;
      background: none;
    }
    .person.selected {
      border-color: var(--primary);
    }
    .add {
      width: 40px;
      height: 40px;
      align-items: center;
      justify-content: center;
      border: 1px dashed var(--border);
      color: var(--muted);
    }
    .add:hover {
      color: var(--fg);
    }
    .badge {
      position: absolute;
      top: -4px;
      right: -6px;
      min-width: 18px;
      padding: 1px 4px;
      border-radius: var(--radius-pill);
      background: var(--danger);
      color: var(--on-avatar);
      font-size: 11px;
      font-weight: 600;
      line-height: 16px;
      text-align: center;
    }
    .dot {
      position: absolute;
      right: -1px;
      bottom: -1px;
      width: 11px;
      height: 11px;
      border-radius: 50%;
      background: var(--success);
      border: 2px solid var(--grouped);
    }
  `,
})
export class RailComponent {
  readonly people = input.required<readonly PersonView[]>();
  readonly items = input.required<readonly Item[]>();
  readonly me = input<string>();
  readonly opened = input<Readonly<Record<string, string>>>({});
  readonly admin = input(false);
  readonly selected = input<string>();
  readonly choose = output<string | undefined>();
  readonly invite = output();

  protected readonly mePerson = computed(() => this.people().find((p) => p.user === this.me()));
  protected readonly others = computed(() => railPeople(this.people(), this.items(), this.me()));

  protected color(user: string): string {
    return avatarColor(user);
  }

  protected initialsOf(name: string): string {
    return initials(name);
  }

  protected count(user: string): number {
    return this.selected() === user ? 0 : unread(this.items(), user, this.opened());
  }

  protected unreadLabel(user: string): string {
    const n = this.count(user);
    return n ? `, ${n} non ${n > 1 ? 'lus' : 'lu'}` : '';
  }

  protected online(p: PersonView): boolean {
    return p.devices.some((d) => d.online);
  }
}
