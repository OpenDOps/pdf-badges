import type { PageDocument } from '../page-view';
import type { PageHistory } from '../page-history';
import type { ToolEffect } from './sheet-tool';

export type ApplyResult =
  | { kind: 'none' }
  | { kind: 'edit-text'; id: string; client: { x: number; y: number } }
  | { kind: 'appended'; id: string }
  | { kind: 'set-foreground'; hex: string }
  | { kind: 'select-entries'; ids: string[] };

/**
 * Record history and run an append, or hand an edit / sample / selection back to the editor.
 * The caller inserts the new box and marks dirty for an append.
 */
export function applyToolEffect(effect: ToolEffect, page: PageDocument, history: PageHistory): ApplyResult {
  if (effect.kind === 'none') return { kind: 'none' };
  if (effect.kind === 'edit-text') return { kind: 'edit-text', id: effect.id, client: effect.client };
  if (effect.kind === 'set-foreground') return { kind: 'set-foreground', hex: effect.hex };
  if (effect.kind === 'select-entries') return { kind: 'select-entries', ids: effect.ids };
  history.record(page);
  const entry = effect.run(page);
  return { kind: 'appended', id: entry.id };
}
