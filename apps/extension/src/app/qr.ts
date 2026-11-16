import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';

import type { QrView } from '../shared/api';

/** QR code dessiné en SVG (modules calculés par le WebAssembly). */
@Component({
  selector: 'app-qr',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <svg
      [attr.viewBox]="viewBox()"
      shape-rendering="crispEdges"
      role="img"
      [attr.aria-label]="label()"
    >
      <rect
        [attr.x]="-4"
        [attr.y]="-4"
        [attr.width]="qr().size + 8"
        [attr.height]="qr().size + 8"
        fill="#fff"
      />
      <path [attr.d]="path()" fill="#000" />
    </svg>
  `,
  styles: `
    svg {
      display: block;
      width: 100%;
      height: auto;
      border-radius: 8px;
    }
  `,
})
export class QrComponent {
  readonly qr = input.required<QrView>();
  readonly label = input('QR code');

  protected readonly viewBox = computed(() => {
    const s = this.qr().size;
    return `-4 -4 ${s + 8} ${s + 8}`;
  });

  /** Un rectangle d'un module par case noire, en un seul chemin. */
  protected readonly path = computed(() => {
    const { size, modules } = this.qr();
    let d = '';
    for (let y = 0; y < size; y++) {
      for (let x = 0; x < size; x++) {
        if (modules[y * size + x]) d += `M${x} ${y}h1v1h-1z`;
      }
    }
    return d;
  });
}
