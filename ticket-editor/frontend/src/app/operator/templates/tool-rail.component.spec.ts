import { TestBed } from '@angular/core/testing';

import { applyEntryColor, type PageDocument } from './page-view';
import { createTextStyleCache, textStyleCache } from './text-style';
import { ToolRailComponent } from './tool-rail.component';

describe('tool rail', () => {
  function buttons(element: HTMLElement): HTMLButtonElement[] {
    return Array.from(element.querySelectorAll('button'));
  }

  it('shows the eight tools, Select pressed, at 3.5rem', () => {
    const fixture = TestBed.createComponent(ToolRailComponent);
    fixture.detectChanges();
    const element = fixture.nativeElement as HTMLElement;
    expect(element.className).toContain('w-[3.5rem]');
    const radios = buttons(element).filter((button) => button.getAttribute('role') === 'radio');
    expect(radios.map((button) => button.getAttribute('aria-label'))).toEqual([
      'Select',
      'Select box',
      'Text',
      'Image',
      'Rectangle',
      'QR',
      'Barcode',
      'Colorpicker',
    ]);
    expect(buttons(element)[0].getAttribute('aria-pressed')).toBe('true');
  });

  it('presses Text on click and returns to Select on Escape', () => {
    const fixture = TestBed.createComponent(ToolRailComponent);
    fixture.detectChanges();
    const element = fixture.nativeElement as HTMLElement;
    const text = buttons(element)[2];
    text.click();
    fixture.detectChanges();
    expect(text.getAttribute('aria-pressed')).toBe('true');
    expect(fixture.componentInstance.active()).toBe('text');

    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    fixture.detectChanges();
    expect(buttons(element)[0].getAttribute('aria-pressed')).toBe('true');
    expect(fixture.componentInstance.active()).toBe('select');
  });

  it('does not switch tools while a sheet contenteditable is focused', () => {
    const fixture = TestBed.createComponent(ToolRailComponent);
    fixture.detectChanges();
    const frame = document.createElement('iframe');
    const box = document.createElement('div');
    box.setAttribute('contenteditable', 'true');
    Object.defineProperty(box, 'isContentEditable', { value: true });
    // jsdom does not focus into iframes; stub the frame document the rail reads.
    Object.defineProperty(frame, 'contentDocument', {
      configurable: true,
      value: { activeElement: box },
    });
    document.body.appendChild(frame);

    document.dispatchEvent(new KeyboardEvent('keydown', { key: 't', bubbles: true }));
    fixture.detectChanges();
    expect(fixture.componentInstance.active()).toBe('select');
    expect(buttons(fixture.nativeElement as HTMLElement)[0].getAttribute('aria-pressed')).toBe('true');
    frame.remove();
  });

  it('disables the create tools on a read-only sheet', () => {
    const fixture = TestBed.createComponent(ToolRailComponent);
    fixture.componentRef.setInput('readonly', true);
    fixture.detectChanges();
    const radios = buttons(fixture.nativeElement as HTMLElement).filter((button) => button.getAttribute('role') === 'radio');
    const label = (button: HTMLButtonElement) => button.getAttribute('aria-label');
    expect(radios.filter((button) => button.disabled).map(label)).toEqual(['Text', 'Image', 'Rectangle', 'QR', 'Barcode']);
    expect(radios.filter((button) => !button.disabled).map(label)).toEqual(['Select', 'Select box', 'Colorpicker']);
  });

  it('opens the foreground dialog and stores #FF6A00 on the text and in the style cache', async () => {
    localStorage.removeItem('ticket-editor:text-style');
    textStyleCache.remember({ font: 'text', fontSizePt: 12, bold: false, italic: false, underline: false, foreground: '#000000', background: '#FFFFFF' });
    const fixture = TestBed.createComponent(ToolRailComponent);
    fixture.componentRef.setInput('foreground', '#000000');
    fixture.componentRef.setInput('background', '#FFFFFF');
    const picks: { target: string; hex: string }[] = [];
    fixture.componentInstance.colorPick.subscribe((pick) => picks.push(pick));
    fixture.detectChanges();
    const element = fixture.nativeElement as HTMLElement;

    (element.querySelector('[aria-label="Foreground"]') as HTMLButtonElement).click();
    fixture.detectChanges();
    await new Promise((resolve) => setTimeout(resolve, 0));
    fixture.detectChanges();
    expect(element.querySelector('[role="dialog"]')).toBeTruthy();
    const box = element.querySelector('[aria-label="Saturation and value"]') as HTMLElement;
    expect(box).toBeTruthy();
    const knob = () => box.querySelector('span') as HTMLElement;
    expect(knob().style.left).toBe('0px');
    expect(knob().style.top).toBe('180px');
    box.getBoundingClientRect = () => ({ left: 0, top: 0, width: 180, height: 180, right: 180, bottom: 180, x: 0, y: 0, toJSON() { return {}; } });
    box.dispatchEvent(new MouseEvent('mousedown', { clientX: 180, clientY: 0, bubbles: true }));
    fixture.detectChanges();
    expect(knob().style.left).toBe('180px');
    expect(knob().style.top).toBe('0px');
    expect(picks.at(-1)).toEqual({ target: 'foreground', hex: '#FF0000' });

    (element.querySelector('.color-backdrop') as HTMLElement).dispatchEvent(new MouseEvent('mousedown', { bubbles: true }));
    fixture.detectChanges();
    expect(element.querySelector('[role="dialog"]')).toBeNull();
    expect(picks.at(-1)).toEqual({ target: 'foreground', hex: '#000000' });
    expect(textStyleCache.read().foreground).toBe('#000000');

    (element.querySelector('[aria-label="Foreground"]') as HTMLButtonElement).click();
    fixture.detectChanges();
    await new Promise((resolve) => setTimeout(resolve, 0));
    fixture.detectChanges();
    const hex = element.querySelector('[aria-label="Hex"]') as HTMLInputElement;
    hex.value = 'orange';
    hex.dispatchEvent(new Event('input'));
    fixture.detectChanges();
    expect(picks.at(-1)?.hex).toBe('#000000');

    hex.value = '#FF6A00';
    hex.dispatchEvent(new Event('input'));
    fixture.detectChanges();
    expect(picks.at(-1)).toEqual({ target: 'foreground', hex: '#FF6A00' });
    await new Promise((resolve) => setTimeout(resolve, 0));
    fixture.detectChanges();

    (element.querySelector('[role="tab"][aria-selected="false"]') as HTMLButtonElement).click();
    fixture.detectChanges();
    await new Promise((resolve) => setTimeout(resolve, 0));
    fixture.detectChanges();
    expect((element.querySelector('[aria-label="Hex"]') as HTMLInputElement).value).toBe('#FF6A00');
    expect(element.querySelector('[aria-label="Hue and saturation"]')).toBeTruthy();
    (element.querySelector('.apply') as HTMLButtonElement).click();
    fixture.detectChanges();
    expect(element.querySelector('[role="dialog"]')).toBeNull();
    expect(textStyleCache.read().foreground).toBe('#FF6A00');

    const page: PageDocument = {
      page_size: { width: 100, height: 100, units: 'mm' },
      resources: { fonts: { text: { font_family: 'Liberation Serif' } }, images: {} },
      contents: [{
        id: 'p1-t1',
        posX: 1,
        posY: 1,
        width: 20,
        height: 10,
        preentered: 'Текст',
        text: { font: 'text', font_size: 4.233, leading: 0, line_height: 5.08 },
      }],
    };
    expect(applyEntryColor(page.contents[0], 'foreground', picks.at(-1)!.hex)).toBe(true);
    expect(page.contents[0].text?.color).toBe('#FF6A00');

    const rect = {
      id: 'p1-t2',
      posX: 5,
      posY: 5,
      width: 20,
      height: 10,
      preentered: '',
      background: '#FFFFFF',
      text: { font: 'text', font_size: 4.233, leading: 0, line_height: 5.08 },
    };
    page.contents.push(rect);
    (element.querySelector('[aria-label="Background"]') as HTMLButtonElement).click();
    fixture.detectChanges();
    await new Promise((resolve) => setTimeout(resolve, 0));
    const backgroundHex = element.querySelector('[aria-label="Hex"]') as HTMLInputElement;
    backgroundHex.value = '#112233';
    backgroundHex.dispatchEvent(new Event('input'));
    fixture.detectChanges();
    (element.querySelector('.apply') as HTMLButtonElement).click();
    fixture.detectChanges();
    expect(applyEntryColor(rect, 'background', '#112233')).toBe(true);
    expect(rect.background).toBe('#112233');

    const stored = createTextStyleCache({
      getItem: (key) => localStorage.getItem(key),
      setItem: (key, value) => localStorage.setItem(key, value),
    });
    expect(stored.read().foreground).toBe('#FF6A00');
    expect(stored.read().background).toBe('#112233');
    expect(stored.read().font).toBe('text');
  });

  it('paints foreground over background and swaps the two colors', () => {
    localStorage.removeItem('ticket-editor:text-style');
    textStyleCache.remember({ font: 'text', fontSizePt: 12, bold: false, italic: false, underline: false, foreground: '#000000', background: '#FFFFFF' });
    const fixture = TestBed.createComponent(ToolRailComponent);
    fixture.componentRef.setInput('foreground', '#112233');
    fixture.componentRef.setInput('background', '#FF6A00');
    const picks: { target: string; hex: string }[] = [];
    fixture.componentInstance.colorPick.subscribe((pick) => picks.push(pick));
    fixture.detectChanges();
    const element = fixture.nativeElement as HTMLElement;
    const background = element.querySelector('[aria-label="Background"]') as HTMLElement;
    const foreground = element.querySelector('[aria-label="Foreground"]') as HTMLElement;
    expect(background.compareDocumentPosition(foreground) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(getComputedStyle(foreground).zIndex).toBe('2');
    expect(getComputedStyle(background).zIndex).toBe('1');
    expect(getComputedStyle(foreground).top).toBe('0px');
    expect(getComputedStyle(background).right).toBe('0px');
    expect(getComputedStyle(background).bottom).toBe('0px');

    (element.querySelector('[aria-label="Swap colors"]') as HTMLButtonElement).click();
    fixture.detectChanges();
    expect(picks).toEqual([
      { target: 'foreground', hex: '#FF6A00' },
      { target: 'background', hex: '#112233' },
    ]);
    expect(textStyleCache.read().foreground).toBe('#FF6A00');
    expect(textStyleCache.read().background).toBe('#112233');
    expect(textStyleCache.read().font).toBe('text');
  });
});
