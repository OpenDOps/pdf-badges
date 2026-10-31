import type { PageDocument, PageEntry } from './page-view';

const LIMIT = 100;

export interface HistoryKey {
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
}

export interface HistoryAvailability {
  undo: boolean;
  redo: boolean;
}

/**
 * Snapshot for undo/redo. `contents` are deep-cloned so later in-place edits on
 * the live page do not rewrite history. `page_size` and `resources` are copied
 * as shells; their value objects are shared and treated as replace-on-write.
 */
export function clonePage(page: PageDocument): PageDocument {
  return {
    page_size: {
      ...page.page_size,
      ...(page.page_size.bleeds ? { bleeds: { ...page.page_size.bleeds } } : {}),
    },
    resources: {
      ...(page.resources.images ? { images: { ...page.resources.images } } : {}),
      ...(page.resources.fonts ? { fonts: { ...page.resources.fonts } } : {}),
    },
    contents: page.contents.map((entry) => cloneEntry(entry)),
  };
}

function cloneEntry(entry: PageEntry): PageEntry {
  return structuredClone(entry);
}

/**
 * The one history the sheet uses. A created box, a delete, a move, and a
 * resize are snapshots of the page object. While a text draft is open, that
 * field keeps its own undo; leaving the text folds the visit into one snapshot here.
 */
export class PageHistory {
  private undoStack: PageDocument[] = [];
  private redoStack: PageDocument[] = [];

  get canUndo(): boolean {
    return this.undoStack.length > 0;
  }

  get canRedo(): boolean {
    return this.redoStack.length > 0;
  }

  clear(): void {
    this.undoStack = [];
    this.redoStack = [];
  }

  /** The page as it was before the change. A new change drops redo. */
  record(before: PageDocument): void {
    this.undoStack.push(clonePage(before));
    if (this.undoStack.length > LIMIT) this.undoStack.shift();
    this.redoStack = [];
  }

  undo(current: PageDocument): PageDocument | null {
    const previous = this.undoStack.pop();
    if (!previous) return null;
    this.redoStack.push(clonePage(current));
    return previous;
  }

  redo(current: PageDocument): PageDocument | null {
    const next = this.redoStack.pop();
    if (!next) return null;
    this.undoStack.push(clonePage(current));
    return next;
  }
}

/** The entry this step brought back or edited. A step that only removes a box has none. */
export function changedEntryId(from: PageDocument, to: PageDocument): string | null {
  const previous = new Map(from.contents.map((entry) => [entry.id, entry]));
  for (const entry of to.contents) {
    if (!previous.has(entry.id)) return entry.id;
  }
  for (const entry of to.contents) {
    const before = previous.get(entry.id);
    if (before && JSON.stringify(before) !== JSON.stringify(entry)) return entry.id;
  }
  return null;
}

/** ⌘Z / Ctrl+Z undoes. ⌘⇧Z, Ctrl+Shift+Z, and Ctrl+Y redo. */
export function historyShortcut(event: HistoryKey): 'undo' | 'redo' | null {
  if (event.altKey || !(event.metaKey || event.ctrlKey)) return null;
  const key = event.key.toLowerCase();
  if (key === 'z' && !event.shiftKey) return 'undo';
  if ((key === 'z' && event.shiftKey) || (key === 'y' && !event.shiftKey)) return 'redo';
  return null;
}

/** The shortcut walks this stack when that direction still has a snapshot. */
export function pageHistoryCommand(event: HistoryKey, page: HistoryAvailability): 'undo' | 'redo' | null {
  const kind = historyShortcut(event);
  if (!kind || !page[kind]) return null;
  return kind;
}
