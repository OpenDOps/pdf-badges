import { linesToBreaks } from '../text-style';
import { TextEditSession, placeCaret } from './text-edit';

describe('text edit session', () => {
  function open(source = 'Hello<br>World', shown = 'Hello\nWorld') {
    const commits: { id: string; value: string }[] = [];
    const session = new TextEditSession((edit) => commits.push(edit));
    const el = document.createElement('div');
    el.tabIndex = 0;
    el.style.fontFamily = 'Liberation Serif';
    el.textContent = shown;
    el.setAttribute('data-preentered', source);
    Object.defineProperty(el, 'innerText', {
      configurable: true,
      get() {
        return el.textContent ?? '';
      },
    });
    document.body.appendChild(el);
    return { session, el, commits };
  }

  afterEach(() => {
    document.body.replaceChildren();
  });

  it('begins typing in the box in its own font', () => {
    const { session, el } = open('Раз<br>Два', 'Раз\nДва');
    session.begin({ el, id: 'p1-t1', source: 'Раз<br>Два' });
    expect(session.editingElement()).toBe(el);
    expect(el.getAttribute('contenteditable')).toBe('true');
    expect(el.textContent).toBe('Раз\nДва');
    expect(el.style.fontFamily).toContain('Liberation Serif');
  });

  it('places the caret on the glyph and only moves it on the same box', () => {
    const { session, el } = open('ABCDEF', 'ABCDEF');
    session.begin({ el, id: 'p1-t1', source: 'ABCDEF' });
    const node = el.firstChild as Text;
    document.caretRangeFromPoint = (_x: number, _y: number) => {
      const hit = document.createRange();
      hit.setStart(node, 3);
      hit.collapse(true);
      return hit;
    };

    placeCaret(el, 10, 1);
    expect(document.getSelection()?.getRangeAt(0).startOffset).toBe(3);

    session.begin({ el, id: 'p1-t1', source: 'ABCDEF', client: { x: 10, y: 1 } });
    expect(session.editingElement()).toBe(el);
    expect(el.getAttribute('contenteditable')).toBe('true');
    expect(document.getSelection()?.getRangeAt(0).startOffset).toBe(3);
  });

  it('inserts a newline on Enter and stays open', () => {
    const { session, el } = open('Hi', 'Hi');
    session.begin({ el, id: 'p1-t1', source: 'Hi' });
    const sel = document.getSelection()!;
    const range = document.createRange();
    range.selectNodeContents(el);
    range.collapse(false);
    sel.removeAllRanges();
    sel.addRange(range);
    el.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
    expect(el.getAttribute('contenteditable')).toBe('true');
    const shown = el.innerText || el.textContent || '';
    expect(shown.includes('\n') || !!el.querySelector('br')).toBe(true);
  });

  it('puts the caret after an Enter when the selection was outside the box', () => {
    const { session, el } = open('Hi', 'Hi');
    session.begin({ el, id: 'p1-t1', source: 'Hi' });
    document.getSelection()?.removeAllRanges();
    el.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
    const sel = document.getSelection()!;
    expect(sel.rangeCount).toBe(1);
    const range = sel.getRangeAt(0);
    expect(el.contains(range.startContainer)).toBe(true);
    range.insertNode(document.createTextNode('Next'));
    const shown = el.innerText || el.textContent || '';
    expect(shown).toContain('Hi');
    expect(shown).toContain('Next');
  });

  it('commits a change on blur once and arms the place guard', () => {
    vi.useFakeTimers();
    const { session, el, commits } = open('Hi', 'Hi');
    session.begin({ el, id: 'p1-t1', source: 'Hi' });
    el.textContent = 'Hello';
    el.dispatchEvent(new FocusEvent('blur', { bubbles: false }));
    expect(commits).toEqual([{ id: 'p1-t1', value: linesToBreaks('Hello') }]);
    expect(el.getAttribute('contenteditable')).toBeNull();
    expect(session.placeGuard()).toBe(true);
    el.dispatchEvent(new FocusEvent('blur', { bubbles: false }));
    expect(commits).toHaveLength(1);
    vi.advanceTimersByTime(300);
    expect(session.placeGuard()).toBe(false);
    vi.useRealTimers();
  });

  it('restores the opening string when the text did not change', () => {
    const { session, el, commits } = open('Hi', 'Shown');
    session.begin({ el, id: 'p1-t1', source: 'Hi' });
    expect(el.textContent).toBe('Hi');
    el.dispatchEvent(new FocusEvent('blur', { bubbles: false }));
    expect(commits).toEqual([]);
    expect(el.textContent).toBe('Shown');
  });

  it('restores the opening string on Escape', () => {
    const { session, el, commits } = open('Hi', 'Shown');
    session.begin({ el, id: 'p1-t1', source: 'Hi' });
    el.textContent = 'Changed';
    el.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
    expect(commits).toEqual([]);
    expect(el.textContent).toBe('Shown');
    expect(el.getAttribute('contenteditable')).toBeNull();
  });

  it('finish keeps a change when the tool leaves without a blur', () => {
    const { session, el, commits } = open('Hi', 'Hi');
    session.begin({ el, id: 'p1-t1', source: 'Hi' });
    el.textContent = 'Kept';
    session.finish();
    expect(commits).toEqual([{ id: 'p1-t1', value: linesToBreaks('Kept') }]);
    expect(el.getAttribute('contenteditable')).toBeNull();
    expect(session.editingElement()).toBeNull();
  });

  it('dispose keeps a change before the canvas is torn down', () => {
    const { session, el, commits } = open('Hi', 'Hi');
    session.begin({ el, id: 'p1-t1', source: 'Hi' });
    el.textContent = 'Saved';
    session.dispose();
    expect(commits).toEqual([{ id: 'p1-t1', value: linesToBreaks('Saved') }]);
    expect(el.getAttribute('contenteditable')).toBeNull();
  });
});
