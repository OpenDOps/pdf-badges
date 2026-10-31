import { PageHistory } from '../page-history';
import type { PageDocument } from '../page-view';
import { applyToolEffect } from './apply-effect';
import type { ToolEffect } from './sheet-tool';

function page(contents: PageDocument['contents'] = []): PageDocument {
  return {
    page_size: { width: 100, height: 100, units: 'mm' },
    resources: { fonts: { text: { font_family: 'Liberation Serif' } } },
    contents: structuredClone(contents),
  };
}

describe('applyToolEffect', () => {
  it('records one history step for an append and names the new id', () => {
    const sheet = page([{ id: 'p1-t1', posX: 0, posY: 0, width: 10, height: 10 }]);
    const before = structuredClone(sheet);
    const history = new PageHistory();
    const effect: ToolEffect = {
      kind: 'append',
      run: (doc) => {
        const entry = { id: 'p1-t2', posX: 2, posY: 3, width: 8, height: 4 };
        doc.contents.push(entry);
        return entry;
      },
    };
    expect(applyToolEffect(effect, sheet, history)).toEqual({ kind: 'appended', id: 'p1-t2' });
    expect(sheet.contents.map((entry) => entry.id)).toEqual(['p1-t1', 'p1-t2']);
    expect(history.canUndo).toBe(true);
    expect(history.undo(sheet)).toEqual(before);
  });

  it('does not record history for an edit-text effect', () => {
    const sheet = page();
    const history = new PageHistory();
    const effect: ToolEffect = { kind: 'edit-text', id: 'p1-t1', client: { x: 4, y: 5 } };
    expect(applyToolEffect(effect, sheet, history)).toEqual({ kind: 'edit-text', id: 'p1-t1', client: { x: 4, y: 5 } });
    expect(history.canUndo).toBe(false);
    expect(sheet.contents).toEqual([]);
  });

  it('leaves the page alone for none', () => {
    const sheet = page([{ id: 'p1-t1', posX: 0, posY: 0, width: 1, height: 1 }]);
    const history = new PageHistory();
    expect(applyToolEffect({ kind: 'none' }, sheet, history)).toEqual({ kind: 'none' });
    expect(history.canUndo).toBe(false);
    expect(sheet.contents).toHaveLength(1);
  });

  it('does not record history for a set-foreground sample', () => {
    const sheet = page([{ id: 'p1-t1', posX: 0, posY: 0, width: 1, height: 1 }]);
    const history = new PageHistory();
    expect(applyToolEffect({ kind: 'set-foreground', hex: '#112233' }, sheet, history)).toEqual({
      kind: 'set-foreground',
      hex: '#112233',
    });
    expect(history.canUndo).toBe(false);
    expect(sheet.contents).toHaveLength(1);
  });

  it('does not record history for a select-entries band', () => {
    const sheet = page([{ id: 'p1-t1', posX: 0, posY: 0, width: 1, height: 1 }]);
    const history = new PageHistory();
    expect(applyToolEffect({ kind: 'select-entries', ids: ['a', 'b'] }, sheet, history)).toEqual({
      kind: 'select-entries',
      ids: ['a', 'b'],
    });
    expect(history.canUndo).toBe(false);
    expect(sheet.contents).toHaveLength(1);
  });
});
