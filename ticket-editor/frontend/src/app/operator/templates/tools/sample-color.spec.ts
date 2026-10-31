import type { PageEntry } from '../page-view';
import { entryAtPoint, picturePixel, rgbaToHex, sampleTextForeground } from './sample-color';

function text(id: string, color?: string): PageEntry {
  return {
    id,
    posX: 10,
    posY: 10,
    width: 20,
    height: 10,
    preentered: 'Hi',
    text: { font: 'text', font_size: 4, leading: 0, line_height: 5, ...(color ? { color } : {}) },
  };
}

describe('sample color', () => {
  it('picks the topmost entry that contains the point', () => {
    const under = text('p1-t1');
    const over: PageEntry = { ...text('p1-t2'), posX: 15, posY: 12, width: 10, height: 8 };
    expect(entryAtPoint([under, over], { x: 18, y: 14 })?.id).toBe('p1-t2');
    expect(entryAtPoint([under, over], { x: 11, y: 11 })?.id).toBe('p1-t1');
    expect(entryAtPoint([under], { x: 0, y: 0 })).toBeNull();
  });

  it('samples text.color or #000000, and skips pictures', () => {
    expect(sampleTextForeground(text('p1-t1', '#112233'))).toBe('#112233');
    expect(sampleTextForeground(text('p1-t1'))).toBe('#000000');
    expect(
      sampleTextForeground({
        id: 'p1-i1',
        posX: 0,
        posY: 0,
        width: 10,
        height: 10,
        image: { resource_name: 'logo' },
      }),
    ).toBeNull();
  });

  it('maps a sheet point into picture pixels and drops transparent', () => {
    const entry: PageEntry = {
      id: 'p1-i1',
      posX: 0,
      posY: 0,
      width: 10,
      height: 10,
      image: { resource_name: 'logo' },
    };
    expect(picturePixel(entry, { x: 0, y: 0 }, 100, 50)).toEqual({ x: 0, y: 0 });
    expect(picturePixel(entry, { x: 9.9, y: 9.9 }, 100, 50)).toEqual({ x: 99, y: 49 });
    expect(picturePixel(entry, { x: -1, y: 0 }, 100, 50)).toBeNull();
    expect(rgbaToHex(255, 106, 0, 255)).toBe('#FF6A00');
    expect(rgbaToHex(255, 106, 0, 0)).toBeNull();
  });
});
