import { InjectionToken, Type } from '@angular/core';

import { PagePayload } from '../../core/models/print.models';

/** What a toolbar contribution may read from the open sheet. The editor
 * provides itself. Contributions do their own HTTP. */
export interface EditorSheet {
  currentHtml(): string;
  page(): PagePayload;
  templateId(): number | null;
  templateName(): string;
  unsaved(): boolean;
  showError(message: string | null, line?: number | null): void;
}

export const EDITOR_SHEET = new InjectionToken<EditorSheet>('EDITOR_SHEET');

/** One extra toolbar cluster. `aside` is the optional panel in the
 * workspace row (the zebra bitmap). */
export interface EditorContribution {
  toolbar: Type<unknown>;
  aside?: Type<unknown>;
}

export const EDITOR_CONTRIBUTIONS = new InjectionToken<EditorContribution[]>('EDITOR_CONTRIBUTIONS');
