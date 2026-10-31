import { PageHistory, changedEntryId, clonePage, historyShortcut, pageHistoryCommand, type HistoryKey } from './page-history';
import type { PageDocument } from './page-view';

function page(): PageDocument {
  return {
    page_size: { width: 100, height: 100, units: 'mm' },
    resources: { fonts: {}, images: {} },
    contents: [],
  };
}

function key(partial: Partial<HistoryKey>): HistoryKey {
  return { key: 'z', metaKey: true, ctrlKey: false, shiftKey: false, altKey: false, ...partial };
}

describe('page history', () => {
  it('undo removes the last created entry and redo puts it back', () => {
    const history = new PageHistory();
    const before = page();
    history.record(before);
    const created = structuredClone(before);
    created.contents.push({
      id: 'p1-b1',
      posX: 0,
      posY: 0,
      width: 30,
      height: 12,
      template: true,
      content: null,
      barcode: { type: 'ean13', preentered: '{{code}}' },
    });
    before.contents.push({
      id: 'mutated-after-record',
      posX: 0,
      posY: 0,
      width: 1,
      height: 1,
      text: { font: 'text', font_size: 4, leading: 0, line_height: 5 },
    });

    const restored = history.undo(created);
    expect(restored?.contents).toEqual([]);
    const again = history.redo(restored!);
    expect(again?.contents.map((entry) => entry.id)).toEqual(['p1-b1']);
    expect(again?.contents[0].barcode).toEqual({ type: 'ean13', preentered: '{{code}}' });
  });

  it('clonePage isolates contents from later in-place edits', () => {
    const live = page();
    live.contents.push({
      id: 'p1-t1',
      posX: 1,
      posY: 2,
      width: 10,
      height: 5,
      preentered: 'Hi',
      text: { font: 'text', font_size: 4, leading: 0, line_height: 5 },
    });
    const snap = clonePage(live);
    live.contents[0].preentered = 'Changed';
    live.contents[0].posX = 99;
    live.page_size.width = 50;
    live.resources.fonts = { text: { font_family: 'Other' } };
    expect(snap.contents[0].preentered).toBe('Hi');
    expect(snap.contents[0].posX).toBe(1);
    expect(snap.page_size.width).toBe(100);
    expect(snap.resources.fonts).toEqual({});
  });

  it('undoes text and QR in reverse order, and a new creation drops redo', () => {
    const history = new PageHistory();
    const empty = page();
    history.record(empty);
    const withText = structuredClone(empty);
    withText.contents.push({
      id: 'p1-t1',
      posX: 1,
      posY: 1,
      width: 40,
      height: 10,
      preentered: 'Текст',
      content: null,
      text: { font: 'text', font_size: 4.233, leading: 0, line_height: 5.08 },
    });
    history.record(withText);
    const withQr = structuredClone(withText);
    withQr.contents.push({
      id: 'p1-b1',
      posX: 0,
      posY: 0,
      width: 30,
      height: 12,
      template: true,
      content: null,
      barcode: { type: 'qr', preentered: '{{code}}' },
    });

    expect(history.undo(withQr)?.contents.map((entry) => entry.id)).toEqual(['p1-t1']);
    expect(history.undo(withText)?.contents).toEqual([]);
    expect(history.canUndo).toBe(false);
    const redone = history.redo(empty);
    expect(redone?.contents.map((entry) => entry.id)).toEqual(['p1-t1']);
    history.record(redone!);
    expect(history.canRedo).toBe(false);
    expect(history.redo(redone!)).toBeNull();
  });

  it('names the box an undo or redo brought back or edited', () => {
    const before = page();
    const moved = structuredClone(before);
    moved.contents.push({
      id: 'p1-b1',
      posX: 0,
      posY: 0,
      width: 30,
      height: 12,
      template: true,
      content: null,
      barcode: { type: 'ean13', preentered: '{{code}}' },
    });
    const created = structuredClone(moved);
    created.contents[0] = { ...created.contents[0], posX: 10 };
    created.contents.push({
      id: 'p1-t1',
      posX: 1,
      posY: 1,
      width: 40,
      height: 10,
      preentered: 'Текст',
      content: null,
      text: { font: 'text', font_size: 4.233, leading: 0, line_height: 5.08 },
    });

    expect(changedEntryId(moved, created)).toBe('p1-t1');
    expect(changedEntryId(created, moved)).toBe('p1-b1');
    expect(changedEntryId(before, moved)).toBe('p1-b1');
    expect(changedEntryId(moved, before)).toBeNull();
  });

  it('walks the one page stack for undo and redo', () => {
    const ready = { undo: true, redo: true };
    const empty = { undo: false, redo: false };
    expect(historyShortcut(key({}))).toBe('undo');
    expect(pageHistoryCommand(key({}), ready)).toBe('undo');
    expect(pageHistoryCommand(key({}), empty)).toBeNull();
    expect(pageHistoryCommand(key({ key: 'Z', shiftKey: true }), ready)).toBe('redo');
    expect(pageHistoryCommand(key({ key: 'y', metaKey: false, ctrlKey: true }), ready)).toBe('redo');
    expect(pageHistoryCommand(key({ key: 'y', metaKey: false, ctrlKey: true }), { undo: true, redo: false })).toBeNull();
    expect(historyShortcut(key({ metaKey: false }))).toBeNull();
    expect(historyShortcut(key({ altKey: true }))).toBeNull();
  });
});
