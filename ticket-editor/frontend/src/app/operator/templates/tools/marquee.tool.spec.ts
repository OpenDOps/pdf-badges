import type { DrawnRect } from '../draw-tool';
import type { PageDocument, PageEntry } from '../page-view';
import { defaultTextStyle } from '../text-style';
import { marqueeTool, selectIdsFromBand } from './marquee.tool';
import type { GestureInput, ToolHost } from './sheet-tool';

function page(contents: PageEntry[]): PageDocument {
  return {
    page_size: { width: 100, height: 100, units: 'mm' },
    resources: { fonts: { text: { font_family: 'Liberation Serif' } } },
    contents,
  };
}

function host(sheet: PageDocument, selected: string[] = []): ToolHost {
  return {
    page: () => sheet,
    style: () => defaultTextStyle(),
    measureLine: () => () => 0,
    editingElement: () => null,
    placeGuard: () => false,
    beginTextEdit: () => undefined,
    selection: () => selected,
  };
}

function gesture(rect: DrawnRect | null, shift = false): GestureInput {
  return { click: { x: 0, y: 0 }, rect, shift, client: { x: 0, y: 0 }, target: null, entry: null };
}

/** Three entries: two fully inside a 10×10 band, one that crosses the edge. */
const three: PageEntry[] = [
  { id: 'a', posX: 1, posY: 1, width: 3, height: 3 },
  { id: 'b', posX: 5, posY: 5, width: 3, height: 3 },
  { id: 'edge', posX: 8, posY: 8, width: 4, height: 4 },
];

const band10: DrawnRect = { posX: 0, posY: 0, width: 10, height: 10, anchorX: 0, anchorY: 0 };

describe('selectIdsFromBand', () => {
  it('keeps the two ids fully inside and drops the one that crosses the edge', () => {
    expect(selectIdsFromBand(three, band10, [], false)).toEqual(['a', 'b']);
  });

  it('unions with the current selection when Shift is held', () => {
    expect(selectIdsFromBand(three, band10, ['kept'], true)).toEqual(['kept', 'a', 'b']);
  });

  it('does not duplicate an id already selected under Shift', () => {
    expect(selectIdsFromBand(three, band10, ['a', 'kept'], true)).toEqual(['a', 'kept', 'b']);
  });

  it('returns an empty list when nothing sits inside', () => {
    const miss = { posX: 50, posY: 50, width: 5, height: 5 };
    expect(selectIdsFromBand(three, miss, ['a'], false)).toEqual([]);
  });
});

describe('marquee tool', () => {
  it('selects the two ids fully inside a band of three entries', () => {
    const sheet = page(three);
    expect(marqueeTool.onGesture(gesture(band10), host(sheet))).toEqual({
      kind: 'select-entries',
      ids: ['a', 'b'],
    });
    expect(sheet.contents).toHaveLength(3);
  });

  it('unions with the host selection when Shift is held', () => {
    const sheet = page(three);
    expect(marqueeTool.onGesture(gesture(band10, true), host(sheet, ['prior']))).toEqual({
      kind: 'select-entries',
      ids: ['prior', 'a', 'b'],
    });
  });

  it('clears the selection when the band contains no id', () => {
    const sheet = page(three);
    const empty = { posX: 40, posY: 40, width: 5, height: 5, anchorX: 40, anchorY: 40 };
    expect(marqueeTool.onGesture(gesture(empty), host(sheet, ['a']))).toEqual({
      kind: 'select-entries',
      ids: [],
    });
  });

  it('returns none when there is no rectangle or no page', () => {
    expect(marqueeTool.onGesture(gesture(null), host(page(three)))).toEqual({ kind: 'none' });
    expect(
      marqueeTool.onGesture(gesture(band10), {
        ...host(page(three)),
        page: () => null,
      }),
    ).toEqual({ kind: 'none' });
  });

  it('does not append an entry', () => {
    const sheet = page(three);
    const before = structuredClone(sheet.contents);
    marqueeTool.onGesture(gesture(band10), host(sheet));
    expect(sheet.contents).toEqual(before);
  });
});
