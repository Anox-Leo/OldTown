import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';

import type { DeviceKind } from './devices';
import { IconComponent } from './icon';

@Component({
  selector: 'app-avatar',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [IconComponent],
  template: `
    @if (kind() === 'person') {
      <span class="initials" [style.font-size.px]="textSize()">{{ initials() }}</span>
    } @else {
      <app-icon [name]="iconName()" [size]="iconSize()" />
    }
  `,
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
      color: var(--on-avatar);
    }
    .initials {
      font-weight: 600;
      letter-spacing: 0.02em;
      line-height: 1;
    }
  `,
})
export class AvatarComponent {
  readonly kind = input.required<DeviceKind>();
  readonly color = input.required<string>();
  readonly size = input(44);
  readonly initials = input('');
  protected readonly textSize = computed(() => Math.round(this.size() * 0.38));
  protected readonly iconName = computed(() => {
    const k = this.kind();
    return k === 'person' ? 'laptop' : k;
  });
  protected readonly iconSize = computed(() => Math.round(this.size() * 0.5));
}
