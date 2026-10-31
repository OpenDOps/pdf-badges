import { PageHistory } from '../page-history';
import type { DrawnRect } from '../draw-tool';
import { ptToMm, type PageDocument, type PageEntry } from '../page-view';
import { defaultTextStyle } from '../text-style';
import { applyToolEffect } from './apply-effect';
import { toolById, type EditorTool } from './registry';
import { entryFromTarget, type GestureInput, type ToolHost } from './sheet-tool';
import { textTool } from './text.tool';

/**
 * Thin harness for the editor's `onToolGesture` path: resolve entry from the
 * live page, run the tool, apply the effect. Does not load GrapesJS.
 */
function page(contents: PageEntry[] = []): PageDocument {
  return {
    page_size: { width: 100, height: 100, units: 'mm' },
    resources: { fonts: { text: { font_family: 'Liberation Serif' } } },
    contents: structuredClone(contents),
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

function host(): ToolHost {
  const style = defaultTextStyle();
  return {
    page: () => null,
    style: () => style,
    measureLine: () => (line) => line.length * ptToMm(style.fontSizePt) * 0.5,
    editingElement: () => null,
    placeGuard: () => false,
    beginTextEdit: () => undefined,
  };
}

function gesture(target: Element | null, entry: PageEntry | null = null): GestureInput {
  return {
    click: { x: 12, y: 8 },
    rect: null as DrawnRect | null,
    shift: false,
    client: { x: 40, y: 22 },
    target,
    entry,
  };
}

function onToolGesture(sheet: PageDocument, history: PageHistory, toolId: EditorTool, input: GestureInput) {
  const entry = entryFromTarget(sheet, input.target);
  const effect = toolById(toolId).onGesture({ ...input, entry }, host());
  return applyToolEffect(effect, sheet, history);
}

describe('onToolGesture harness', () => {
  it('edits the live page entry under the target without recording history', () => {
    const sheet = page([textEntry()]);
    const history = new PageHistory();
    const box = document.createElement('div');
    box.setAttribute('data-yaml-id', 'p1-t1');
    document.body.appendChild(box);

    expect(onToolGesture(sheet, history, 'text', gesture(box))).toEqual({
      kind: 'edit-text',
      id: 'p1-t1',
      client: { x: 40, y: 22 },
    });
    expect(history.canUndo).toBe(false);
    expect(sheet.contents).toHaveLength(1);

    document.body.replaceChildren();
  });

  it('appends when the target has no page entry and records one history step', () => {
    const sheet = page();
    const history = new PageHistory();
    const result = onToolGesture(sheet, history, textTool.id, gesture(null));
    expect(result.kind).toBe('appended');
    if (result.kind !== 'appended') return;
    expect(result.id).toBe('p1-t1');
    expect(sheet.contents).toHaveLength(1);
    expect(history.canUndo).toBe(true);
    expect(history.undo(sheet)?.contents).toEqual([]);
  });
});
