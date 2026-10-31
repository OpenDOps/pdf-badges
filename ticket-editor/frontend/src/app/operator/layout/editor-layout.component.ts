import { Component } from '@angular/core';
import { RouterOutlet } from '@angular/router';

/** Full-screen shell for the template editor (docs/TZ_print_templates.md,
 * раздел 6.2): no operator header — the canvas, palette and property panels
 * need the whole viewport, and the editor's own toolbar carries the way
 * back («← Макеты»). The <main> is a definite-height flex column so the
 * editor's side panels scroll inside themselves instead of growing the
 * page (see TemplateEditorComponent's host class). */
@Component({
  selector: 'app-editor-layout',
  standalone: true,
  imports: [RouterOutlet],
  template: `
    <main class="h-screen box-border flex flex-col bg-bg-base p-4 overflow-hidden">
      <router-outlet />
    </main>
  `,
})
export class EditorLayoutComponent {}
