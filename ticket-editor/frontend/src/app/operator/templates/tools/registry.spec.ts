import { editorToolIds, sheetTools, toolShortcuts, toolsInOrder } from './registry';

describe('tool registry', () => {
  it('sorts the rail Select, Select box, Text, Image, Rectangle, QR, Barcode, Colorpicker', () => {
    expect(toolsInOrder().map((tool) => tool.label)).toEqual([
      'Select',
      'Select box',
      'Text',
      'Image',
      'Rectangle',
      'QR',
      'Barcode',
      'Colorpicker',
    ]);
    expect(editorToolIds).toEqual(['select', 'marquee', 'text', 'image', 'rect', 'qr', 'barcode', 'colorpicker']);
  });

  it('maps the bare keys and leaves Escape to the rail', () => {
    expect(toolShortcuts).toEqual({
      v: 'select',
      m: 'marquee',
      t: 'text',
      i: 'image',
      r: 'rect',
      q: 'qr',
      b: 'barcode',
      e: 'colorpicker',
    });
    expect(toolShortcuts['escape']).toBeUndefined();
    expect(sheetTools.some((tool) => tool.key === 'Escape')).toBe(false);
  });

  it('lets Select, Select box, and Colorpicker stay available on a read-only sheet', () => {
    const creates = Object.fromEntries(sheetTools.map((tool) => [tool.id, tool.creates]));
    expect(creates).toEqual({
      select: false,
      marquee: false,
      text: true,
      image: true,
      rect: true,
      qr: true,
      barcode: true,
      colorpicker: false,
    });
  });

  it('gives each tool the gesture and cursor the pointer session reads', () => {
    const modes = Object.fromEntries(sheetTools.map((tool) => [tool.id, { gesture: tool.gesture, cursor: tool.cursor }]));
    expect(modes).toEqual({
      select: { gesture: 'canvas', cursor: null },
      marquee: { gesture: 'drag', cursor: null },
      text: { gesture: 'click', cursor: 'text' },
      image: { gesture: 'drag', cursor: 'crosshair' },
      rect: { gesture: 'drag', cursor: 'crosshair' },
      qr: { gesture: 'drag', cursor: 'crosshair' },
      barcode: { gesture: 'drag', cursor: 'crosshair' },
      colorpicker: { gesture: 'click', cursor: 'crosshair' },
    });
  });
});
