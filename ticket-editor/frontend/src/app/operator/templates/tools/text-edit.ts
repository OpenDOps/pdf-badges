import { breaksToLines, linesToBreaks } from '../text-style';

/**
 * Type in the box itself. The text tool opens this session; it does not own the DOM.
 * Enter inserts a line. Blur keeps the text. Escape restores the opening string.
 */
export class TextEditSession {
  private el: HTMLElement | null = null;
  private id = '';
  private source = '';
  private shown = '';
  private abort: AbortController | null = null;
  private cancel = false;
  private lock = false;
  private guard = false;
  private guardTimer: ReturnType<typeof setTimeout> | null = null;

  constructor(private readonly onCommit: (edit: { id: string; value: string }) => void = () => undefined) {}

  editingElement(): HTMLElement | null {
    return this.el;
  }

  placeGuard(): boolean {
    return this.guard;
  }

  /**
   * Open the caret in that box. The same element only moves the caret.
   * `source` is the stored `preentered` (`<br>` for newlines).
   */
  begin(args: { el: HTMLElement; id: string; source: string; client?: { x: number; y: number } }): void {
    const { el, id, source, client } = args;
    if (this.el === el) {
      el.focus({ preventScroll: true });
      if (client) placeCaret(el, client.x, client.y);
      return;
    }
    if (this.el) this.el.blur();
    this.id = id;
    this.source = source;
    this.shown = el.textContent ?? '';
    this.el = el;
    this.cancel = false;
    el.textContent = breaksToLines(source);
    el.setAttribute('contenteditable', 'true');
    el.spellcheck = false;
    el.focus({ preventScroll: true });
    if (client) placeCaret(el, client.x, client.y);
    this.abort?.abort();
    const abort = new AbortController();
    this.abort = abort;
    el.addEventListener('keydown', (event) => this.onKey(event), { signal: abort.signal });
    el.addEventListener('blur', () => this.commit(), { signal: abort.signal });
  }

  /**
   * Keep the typed text and close. Used when the tool changes or the canvas leaves.
   * A rail click does not blur the box; this is the commit that click must still get.
   */
  finish(): void {
    if (!this.el) return;
    this.cancel = false;
    this.commit();
  }

  /**
   * Commit any open edit, then drop listeners. Destroy and tool teardown call this
   * so typed text is not stripped without an emit.
   */
  dispose(): void {
    this.finish();
    if (this.guardTimer) clearTimeout(this.guardTimer);
    this.guardTimer = null;
    this.guard = false;
    this.abort?.abort();
    this.abort = null;
    const el = this.el;
    this.el = null;
    if (el) el.removeAttribute('contenteditable');
  }

  private onKey(event: KeyboardEvent): void {
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      this.cancel = true;
      const el = this.el;
      el?.blur();
      // Some test documents do not fire blur when the element was never focused.
      if (this.el) this.commit();
      return;
    }
    if (event.key !== 'Enter' || event.isComposing || event.shiftKey || event.metaKey || event.ctrlKey || event.altKey) return;
    event.preventDefault();
    event.stopPropagation();
    const el = this.el;
    if (!el) return;
    insertLine(el);
  }

  private commit(): void {
    if (this.lock) return;
    const el = this.el;
    if (!el) return;
    if (this.cancel) {
      this.cancel = false;
      el.textContent = this.shown;
      this.detach();
      return;
    }
    const next = readInline(el);
    const id = this.id;
    const changed = next !== this.source;
    if (!changed) el.textContent = this.shown;
    this.detach();
    this.armGuard();
    if (!changed) return;
    this.lock = true;
    this.onCommit({ id, value: next });
    this.lock = false;
  }

  private detach(): void {
    const el = this.el;
    this.abort?.abort();
    this.abort = null;
    this.el = null;
    if (el) el.removeAttribute('contenteditable');
  }

  private armGuard(): void {
    this.guard = true;
    if (this.guardTimer) clearTimeout(this.guardTimer);
    this.guardTimer = setTimeout(() => {
      this.guard = false;
      this.guardTimer = null;
    }, 300);
  }
}

/** The caret sits on the clicked glyph. A click in the empty part of the box goes to the end. */
export function placeCaret(el: HTMLElement, x: number, y: number): void {
  const doc = el.ownerDocument;
  const sel = doc.getSelection();
  if (!sel) return;
  const legacy = doc as Document & { caretRangeFromPoint?: (x: number, y: number) => Range | null };
  let range = legacy.caretRangeFromPoint?.(x, y) ?? null;
  if (range && !el.contains(range.startContainer)) range = null;
  if (!range) {
    const pos = doc.caretPositionFromPoint?.(x, y);
    if (pos && el.contains(pos.offsetNode)) {
      range = doc.createRange();
      range.setStart(pos.offsetNode, pos.offset);
      range.collapse(true);
    }
  }
  if (!range) {
    range = doc.createRange();
    range.selectNodeContents(el);
    range.collapse(false);
  }
  sel.removeAllRanges();
  sel.addRange(range);
}

function insertLine(el: HTMLElement): void {
  const doc = el.ownerDocument;
  el.focus({ preventScroll: true });
  // The engine inserts the break and leaves the caret after it. A hand-built
  // <br> plus setStartAfter looks right in getSelection, but the next
  // insertText from the host still lands before the break.
  if (typeof doc.execCommand === 'function') {
    if (doc.execCommand('insertLineBreak')) return;
    if (doc.execCommand('insertText', false, '\n')) return;
  }
  const sel = doc.getSelection();
  const range = sel && sel.rangeCount > 0 ? sel.getRangeAt(0) : null;
  const br = doc.createElement('br');
  const tail = doc.createTextNode('');
  if (!range || !el.contains(range.startContainer)) {
    el.append(br, tail);
  } else {
    range.deleteContents();
    range.insertNode(br);
    br.after(tail);
  }
  const next = doc.createRange();
  next.setStart(tail, 0);
  next.collapse(true);
  sel?.removeAllRanges();
  sel?.addRange(next);
}

function readInline(el: HTMLElement): string {
  let text = (el.innerText || el.textContent || '').replace(/\u00a0/g, ' ');
  if (text.endsWith('\n') && !(el.textContent ?? '').endsWith('\n')) text = text.slice(0, -1);
  return linesToBreaks(text);
}
