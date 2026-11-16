import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';

import type { DeviceKind } from './devices';
import { IconComponent } from './icon';

/** Avatar rond d'un appareil : pictogramme de son type sur sa couleur (note 0031). */
@Component({
  selector: 'app-avatar',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent],
  template: `<app-icon [name]="kind()" [size]="iconSize()" />`,
  host: {
    '[style.background]': 'color()',
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
      color: #fff;
    }
  `,
})
export class AvatarComponent {
  readonly kind = input.required<DeviceKind>();
  readonly color = input.required<string>();
  readonly size = input(44);
  protected readonly iconSize = computed(() => Math.round(this.size() * 0.5));
}
