import { Routes } from '@angular/router';

import { ListPlaceholder } from './list-placeholder.component';
import { EditorLayoutComponent } from './operator/layout/editor-layout.component';
import { TemplateEditorComponent } from './operator/templates/template-editor.component';

function canLeave(component: { canDeactivate: () => boolean }): boolean {
  return component.canDeactivate();
}

export const routes: Routes = [
  { path: 'editor/list', component: ListPlaceholder },
  { path: 'editor', pathMatch: 'full', redirectTo: '/editor/1' },
  {
    path: 'editor/:id',
    component: EditorLayoutComponent,
    children: [
      {
        path: '',
        component: TemplateEditorComponent,
        data: { localCatalog: true },
        canDeactivate: [canLeave],
      },
    ],
  },
  { path: '**', redirectTo: '/editor/list' },
];
