import type { DrawnRect } from '../draw-tool';
import type { PageDocument } from '../page-view';
import { defaultTextStyle } from '../text-style';
import { toolById } from './registry';
import type { GestureInput, ToolEffect, ToolHost } from './sheet-tool';

function page(): PageDocument {
  return {
    page_size: { width: 100, height: 100, units: 'mm' },
    resources: { fonts: { text: { font_family: 'Liberation Serif' } } },
    contents: [],
  };
}

function host(): ToolHost {
  return {
    page: () => null,
    style: () => defaultTextStyle(),
    measureLine: () => () => 0,
    editingElement: () => null,
    placeGuard: () => false,
    beginTextEdit: () => undefined,
  };
}

function drag(width: number, height: number): DrawnRect {
  return { posX: 2, posY: 4, width, height, anchorX: 2, anchorY: 4 };
}

function gesture(rect: DrawnRect | null, shift = false): GestureInput {
  return { click: { x: 2, y: 4 }, rect, shift, client: { x: 0, y: 0 }, target: null, entry: null };
}

function placed(effect: ToolEffect, sheet: PageDocument) {
  if (effect.kind !== 'append') throw new Error(effect.kind);
  return effect.run(sheet);
}

describe('drag tool', () => {
  it('shares the crosshair drag for rectangle, image, QR, and barcode', () => {
    for (const id of ['rect', 'image', 'qr', 'barcode'] as const) {
      const tool = toolById(id);
      expect(tool.gesture).toBe('drag');
      expect(tool.cursor).toBe('crosshair');
    }
  });

  it('appends one text entry at the rectangle drag', () => {
    const sheet = page();
    const entry = placed(toolById('rect').onGesture(gesture(drag(10, 6)), host()), sheet);
    expect(sheet.contents).toEqual([entry]);
    expect(entry).toMatchObject({ posX: 2, posY: 4, width: 10, height: 6, preentered: '' });
    expect(entry.text).toBeTruthy();
    expect(entry.barcode).toBeUndefined();
  });

  it('locks a QR to a square only while Shift is held, and leaves a barcode as the drag', () => {
    const shifted = drag(10, 6);
    const square = placed(toolById('qr').onGesture(gesture(shifted, true), host()), page());
    expect(square).toMatchObject({ posX: 2, posY: 4, width: 10, height: 10, barcode: { type: 'qr', preentered: '{{code}}' } });

    const plain = placed(toolById('qr').onGesture(gesture(shifted, false), host()), page());
    expect(plain).toMatchObject({ width: 10, height: 6, barcode: { type: 'qr' } });

    const barcode = placed(toolById('barcode').onGesture(gesture(shifted, true), host()), page());
    expect(barcode).toMatchObject({ posX: 2, posY: 4, width: 10, height: 6, barcode: { type: 'ean13', preentered: '{{code}}' } });
  });

  it('returns none when the drag has no rectangle', () => {
    for (const id of ['rect', 'image', 'qr', 'barcode'] as const) {
      expect(toolById(id).onGesture(gesture(null), host())).toEqual({ kind: 'none' });
    }
  });

  it('does not append for an image drag', () => {
    const sheet = page();
    sheet.contents.push({ id: 'p1-t1', posX: 0, posY: 0, width: 10, height: 10 });
    expect(toolById('image').onGesture(gesture(drag(10, 6)), host())).toEqual({ kind: 'none' });
    expect(sheet.contents).toHaveLength(1);
  });

  it('select-box drag selects ids and does not append', () => {
    const sheet = page();
    sheet.contents.push(
      { id: 'in', posX: 2, posY: 4, width: 4, height: 4 },
      { id: 'out', posX: 20, posY: 20, width: 4, height: 4 },
    );
    const withPage: ToolHost = { ...host(), page: () => sheet };
    expect(toolById('marquee').onGesture(gesture(drag(10, 6)), withPage)).toEqual({
      kind: 'select-entries',
      ids: ['in'],
    });
    expect(sheet.contents).toHaveLength(2);
  });
});

