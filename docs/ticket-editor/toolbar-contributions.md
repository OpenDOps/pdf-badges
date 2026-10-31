# Add a print or preview toolbar

[Step 5](implementation-plan.md#step-5-plug-toolbar-actions-into-the-route) took preview, Zebra preview, test print, and export out of `TemplateEditorComponent`. The base screen is the same on `/editor/:id` and `/operator/templates/:id`. A route puts those buttons back by registering a contribution. The editor template does not grow a flag, and the buttons are not pasted back into `template-editor.component.html`.

The types live in `PIR_REG/expocab-offline-registration/frontend/src/app/operator/templates/editor-sheet.ts`. The Django cluster that used to sit in the editor is `django-print.contribution.ts` in the same folder.

## What the editor renders

`TemplateEditorComponent` reads the token once:

```ts
readonly contributions =
  inject(EDITOR_CONTRIBUTIONS, { optional: true }) ?? [];
```

Each `toolbar` is drawn to the left of Save with `NgComponentOutlet`. Each `aside` is drawn in the workspace row, beside the canvas. Both hosts use `display: contents`, so the contribution's own elements sit in the toolbar and the workspace row. An aside draws its panel only while it has something to show. `/editor/:id` provides nothing, so the list is empty and the buttons are absent.

## What a contribution may read

The editor provides itself as `EDITOR_SHEET` (`useExisting: TemplateEditorComponent`). A contribution injects that token and does its own HTTP. It does not reach into the editor's fields.

| Method | What it returns |
|---|---|
| `currentHtml()` | The HTML that would be saved, from the visual canvas or the code editor. The Django cluster uses this. |
| `document()` | The page object the editor holds. The PDF preview contribution uses this. |
| `page()` | The page size payload (`format`, millimetres, orientation, margins) |
| `templateId()` | The open template id, or `null` before the sheet has loaded |
| `templateName()` | The stored name, used for the export filename |
| `unsaved()` | Whether Save is enabled |
| `showError(message, line?)` | The red banner. Pass `null` to clear it. `line` highlights a code-editor line |

`currentHtml()` is the name because the editor already stores the document in an `html` field.

## Put the Django cluster back on a route

`djangoPrintContribution` is the registration-number field, Превью PDF, Как на Zebra, the printer dropdown, Тест-печать, Экспорт, and the mono preview aside. `DjangoPrintState` holds the zebra bitmap URL for both the toolbar and the aside. Provide the state on the same route. It is not `providedIn: 'root'`, so the bitmap does not leak onto `/editor`.

`operator/templates/:id` already has this block in `app.routes.ts`. To show the same cluster on another route, add the same providers there:

```ts
import { DjangoPrintState, djangoPrintContribution } from './operator/templates/django-print.contribution';
import { EDITOR_CONTRIBUTIONS } from './operator/templates/editor-sheet';

{
  path: 'operator/templates/:id',
  providers: [
    DjangoPrintState,
    { provide: EDITOR_CONTRIBUTIONS, multi: true, useValue: djangoPrintContribution },
  ],
  loadComponent: () =>
    import('./operator/layout/editor-layout.component').then((m) => m.EditorLayoutComponent),
}
```

`multi: true` matters. Each `useValue` is one contribution. Angular collects them into the array the editor reads. A second cluster on the same route is another entry in that `providers` array, with its own `useValue`.

Those buttons post to whatever `apiBaseUrl` the build uses. On `npm start` that is `http://localhost:8000/api`. On `npm run start:editor` it is `http://127.0.0.1:4310/api`, and the Node process does not answer preview, test print, export, or `GET /api/printer/list`. Leave this cluster off `/editor/:id`.

## Example: a preview button of your own

A later PDF download is a new contribution, registered only on `/editor/:id`. It does not import `djangoPrintContribution`. This is the shape. The click handler is the only part that changes from one button to the next.

`pdf-preview.contribution.ts`, next to `editor-sheet.ts`:

```ts
import { Component, inject, signal } from '@angular/core';
import { finalize } from 'rxjs';

import { PrintApiService } from '../../core/api/print-api.service';
import { EDITOR_SHEET, EditorContribution } from './editor-sheet';

@Component({
  selector: 'app-pdf-preview-toolbar',
  standalone: true,
  host: { style: 'display: contents' },
  template: `
    <button type="button" class="px-3 py-1.5 rounded-lg bg-button-base text-text-base" [disabled]="busy()" (click)="preview()">
      {{ busy() ? 'Рендер…' : 'Превью PDF' }}
    </button>
  `,
})
export class PdfPreviewToolbarComponent {
  private readonly sheet = inject(EDITOR_SHEET);
  private readonly api = inject(PrintApiService);
  readonly busy = signal(false);

  preview(): void {
    const templateId = this.sheet.templateId();
    if (templateId === null || this.busy()) {
      return;
    }
    this.busy.set(true);
    this.sheet.showError(null);
    this.api
      .preview({
        template_id: templateId,
        document: this.sheet.document(),
        output: 'pdf',
      })
      .pipe(finalize(() => this.busy.set(false)))
      .subscribe({
        next: (blob) => window.open(URL.createObjectURL(blob), '_blank'),
        error: () => this.sheet.showError('Не удалось построить превью.'),
      });
  }
}

export const pdfPreviewContribution: EditorContribution = {
  toolbar: PdfPreviewToolbarComponent,
};
```

Register it on `/editor/:id` only. `PreviewRequest` includes `document`, the page object. The Django cluster does not send that field.

```ts
{
  path: 'editor/:id',
  providers: [
    { provide: EDITOR_CONTRIBUTIONS, multi: true, useValue: pdfPreviewContribution },
  ],
  loadComponent: () =>
    import('./operator/layout/editor-layout.component').then((m) => m.EditorLayoutComponent),
}
```

An aside is optional. Add `aside: YourAsideComponent` on the same object when the toolbar and a panel must share a signal. Provide that state class in the same `providers` array, the way `DjangoPrintState` is provided next to `djangoPrintContribution`. The aside component injects the state, sets `host: { style: 'display: contents' }`, and wraps the panel in `@if` so an empty preview takes no space.

Test print follows the same click handler. Read `this.sheet.templateId()` and `this.sheet.unsaved()`. When there are unsaved edits, confirm before posting, because test print uses the saved template. `PrintApiService.testPrint` is the call. `django-print.contribution.ts` is the working copy of that confirm, the printer dropdown, and `workstation.refresh()`.
