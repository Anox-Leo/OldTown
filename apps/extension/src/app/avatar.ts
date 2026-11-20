import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';

import { MarkComponent } from './brand';
import type { DeviceKind } from './devices';
import { IconComponent } from './icon';

/**
 * Avatar rond d'un appareil : pictogramme de son type sur sa couleur (note
 * 0031) ; « Tous mes appareils » : le signe sur la tuile de l'icône.
 */
@Component({
  selector: 'app-avatar',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent, MarkComponent],
  template: `
    @if (kind() === 'group') {
      <app-mark [size]="markSize()" />
    } @else {
      <app-icon [name]="kind()" [size]="iconSize()" />
    }
  `,
  host: {
    '[style.background]': "kind() === 'group' ? 'var(--tile)' : color()",
    '[style.width.px]': 'size()',
    '[style.height.px]': 'size()',
    'aria-hidden': 'true',
  },
  styles: `
    :host {
      flex: none;
      display: inline-flex;
      align-items: center;
      justify-content: center;
      border-radius: 50%;
      color: var(--on-avatar);
    }
  `,
})
export class AvatarComponent {
  readonly kind = input.required<DeviceKind>();
  readonly color = input.required<string>();
  readonly size = input(44);
  protected readonly iconSize = computed(() => Math.round(this.size() * 0.5));
  protected readonly markSize = computed(() => Math.round(this.size() * 0.55));
}
