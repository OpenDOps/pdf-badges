import type { DrawnRect } from '../draw-tool';
import type { PageDocument, PageEntry } from '../page-view';
import { defaultTextStyle } from '../text-style';
import { colorpickerTool } from './colorpicker.tool';
import type { GestureInput, ToolHost } from './sheet-tool';

function page(contents: PageEntry[]): PageDocument {
  return {
    page_size: { width: 100, height: 100, units: 'mm' },
    resources: { fonts: { text: { font_family: 'Liberation Serif' } } },
    contents,
  };
}

function host(sheet: PageDocument, samplePicture?: ToolHost['samplePicture']): ToolHost {
  return {
    page: () => sheet,
    style: () => defaultTextStyle(),
    measureLine: () => () => 0,
    editingElement: () => null,
    placeGuard: () => false,
    beginTextEdit: () => undefined,
    samplePicture,
  };
}

function click(point: { x: number; y: number }, entry: PageEntry | null = null): GestureInput {
  return {
    click: point,
    rect: null as DrawnRect | null,
    shift: false,
    client: { x: 0, y: 0 },
    target: null,
    entry,
  };
}

describe('colorpicker tool', () => {
  it('samples text.color into set-foreground and leaves the entry alone', () => {
    const entry: PageEntry = {
      id: 'p1-t1',
      posX: 5,
      posY: 5,
      width: 20,
      height: 10,
      preentered: 'Hi',
      text: { font: 'text', font_size: 4, leading: 0, line_height: 5, color: '#112233' },
    };
    const sheet = page([entry]);
    expect(colorpickerTool.onGesture(click({ x: 10, y: 8 }), host(sheet))).toEqual({
      kind: 'set-foreground',
      hex: '#112233',
    });
    expect(entry.text?.color).toBe('#112233');
  });

  it('stores #000000 when a text box has no color', () => {
    const sheet = page([
      {
        id: 'p1-t1',
        posX: 0,
        posY: 0,
        width: 10,
        height: 10,
        preentered: '',
        background: '#FFFFFF',
        text: { font: 'text', font_size: 4, leading: 0, line_height: 5 },
      },
    ]);
    expect(colorpickerTool.onGesture(click({ x: 2, y: 2 }), host(sheet))).toEqual({
      kind: 'set-foreground',
      hex: '#000000',
    });
  });

  it('samples a picture pixel through the host', () => {
    const sheet = page([
      {
        id: 'p1-i1',
        posX: 0,
        posY: 0,
        width: 30,
        height: 12,
        image: { resource_name: 'logo' },
      },
    ]);
    const samples: string[] = [];
    const effect = colorpickerTool.onGesture(
      click({ x: 4, y: 4 }),
      host(sheet, (id, point) => {
        samples.push(`${id}@${point.x},${point.y}`);
        return '#FF6A00';
      }),
    );
    expect(samples).toEqual(['p1-i1@4,4']);
    expect(effect).toEqual({ kind: 'set-foreground', hex: '#FF6A00' });
  });

  it('leaves foreground alone on empty sheet or a transparent pixel', () => {
    expect(colorpickerTool.onGesture(click({ x: 1, y: 1 }), host(page([])))).toEqual({ kind: 'none' });
    const sheet = page([
      {
        id: 'p1-i1',
        posX: 0,
        posY: 0,
        width: 10,
        height: 10,
        image: { resource_name: 'logo' },
      },
    ]);
    expect(colorpickerTool.onGesture(click({ x: 2, y: 2 }), host(sheet, () => null))).toEqual({ kind: 'none' });
  });
});
