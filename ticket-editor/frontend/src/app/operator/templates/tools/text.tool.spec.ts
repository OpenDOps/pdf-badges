import type { DrawnRect } from '../draw-tool';
import { ptToMm, type PageDocument, type PageEntry } from '../page-view';
import { defaultTextStyle } from '../text-style';
import type { GestureInput, ToolHost } from './sheet-tool';
import { textTool } from './text.tool';

function page(): PageDocument {
  return {
    page_size: { width: 100, height: 100, units: 'mm' },
    resources: { fonts: { text: { font_family: 'Liberation Serif' } } },
    contents: [],
  };
}

function textEntry(id = 'p1-t1'): PageEntry {
  return {
    id,
    posX: 10,
    posY: 20,
    width: 40,
    height: 10,
    preentered: 'Текст',
    text: { font: 'text', font_size: 4.233, leading: 0, line_height: 5.08 },
  };
}

function host(overrides: Partial<ToolHost> = {}): ToolHost {
  const style = defaultTextStyle();
  return {
    page: () => null,
    style: () => style,
    measureLine: () => (line) => line.length * ptToMm(style.fontSizePt) * 0.5,
    editingElement: () => null,
    placeGuard: () => false,
    beginTextEdit: () => undefined,
    ...overrides,
  };
}

function click(entry: PageEntry | null = null, client = { x: 12, y: 8 }): GestureInput {
  return {
    click: { x: 12.2, y: 7.4 },
    rect: null as DrawnRect | null,
    shift: false,
    client,
    target: null,
    entry,
  };
}

describe('text tool', () => {
  it('appends Текст at the click when the sheet is empty', () => {
    const sheet = page();
    const effect = textTool.onGesture(click(null), host());
    expect(effect.kind).toBe('append');
    if (effect.kind !== 'append') return;
    const entry = effect.run(sheet);
    expect(sheet.contents).toEqual([entry]);
    expect(entry.id).toBe('p1-t1');
    expect(entry.preentered).toBe('Текст');
    expect(entry.posX).toBe(12);
    expect(entry.height).toBeGreaterThan(0);
    expect(entry.posY + entry.height).toBe(7.5);
  });

  it('edits a text entry and does not append', () => {
    const sheet = page();
    const entry = textEntry();
    const effect = textTool.onGesture(click(entry, { x: 40, y: 22 }), host());
    expect(effect).toEqual({ kind: 'edit-text', id: 'p1-t1', client: { x: 40, y: 22 } });
    expect(sheet.contents).toEqual([]);
  });

  it('appends when the click lands on an image or a code', () => {
    const image: PageEntry = { id: 'p1-i1', posX: 0, posY: 0, width: 10, height: 10, image: { resource_name: 'logo' } };
    const code: PageEntry = { id: 'p1-b1', posX: 0, posY: 0, width: 10, height: 10, barcode: { type: 'qr', preentered: '{{code}}' } };
    for (const entry of [image, code]) {
      const sheet = page();
      const effect = textTool.onGesture(click(entry), host());
      expect(effect.kind).toBe('append');
      if (effect.kind !== 'append') continue;
      effect.run(sheet);
      expect(sheet.contents).toHaveLength(1);
      expect(sheet.contents[0].preentered).toBe('Текст');
    }
  });

  it('keeps the place guard from appending on empty sheet', () => {
    expect(textTool.onGesture(click(null), host({ placeGuard: () => true }))).toEqual({ kind: 'none' });
    expect(textTool.onGesture(click(textEntry('p1-t2')), host({ placeGuard: () => true }))).toEqual({
      kind: 'edit-text',
      id: 'p1-t2',
      client: { x: 12, y: 8 },
    });
  });

  it('yields the pointer inside the box being typed and blurs outside', () => {
    const box = document.createElement('div');
    const outside = document.createElement('div');
    box.tabIndex = 0;
    document.body.append(box, outside);
    const blurred: string[] = [];
    box.addEventListener('blur', () => blurred.push('blur'));
    const editing = host({ editingElement: () => box });
    const down = (target: EventTarget) => {
      const event = new PointerEvent('pointerdown', { bubbles: true, cancelable: true });
      Object.defineProperty(event, 'target', { value: target });
      return event;
    };

    expect(textTool.onPointerDown?.(down(document.body), host())).toBe('take');
    expect(textTool.onPointerDown?.(down(box), editing)).toBe('yield');
    expect(blurred).toEqual([]);

    box.focus();
    expect(textTool.onPointerDown?.(down(outside), editing)).toBe('take');
    expect(blurred).toEqual(['blur']);
    document.body.replaceChildren();
  });
});

