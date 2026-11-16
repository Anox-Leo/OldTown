import { TestBed } from '@angular/core/testing';

import { QrComponent } from './qr';

describe('QrComponent', () => {
  it('dessine un carré par module noir, sur fond blanc', () => {
    const fixture = TestBed.createComponent(QrComponent);
    fixture.componentRef.setInput('qr', { size: 2, modules: [1, 0, 0, 1] });
    fixture.detectChanges();
    const svg = fixture.nativeElement.querySelector('svg') as SVGElement;
    expect(svg.getAttribute('viewBox')).toBe('-4 -4 10 10');
    expect(svg.querySelector('path')?.getAttribute('d')).toBe('M0 0h1v1h-1zM1 1h1v1h-1z');
  });
});
