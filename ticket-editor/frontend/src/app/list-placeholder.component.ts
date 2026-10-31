import { HttpClient } from '@angular/common/http';
import { Component, inject, signal } from '@angular/core';
import { RouterLink } from '@angular/router';

export interface TemplateRow {
  id: number | string;
  name: string;
  updated_at: string;
  page_width_mm: number;
  page_height_mm: number;
}

@Component({
  selector: 'app-list-placeholder',
  imports: [RouterLink],
  styleUrl: './list-placeholder.component.scss',
  templateUrl: './list-placeholder.component.html',
})
export class ListPlaceholder {
  private readonly http = inject(HttpClient);
  readonly templates = signal<TemplateRow[]>([]);
  readonly error = signal(false);

  constructor() {
    this.http.get<TemplateRow[]>('/api/print_template/list').subscribe({
      next: (rows) => this.templates.set(rows),
      error: () => this.error.set(true),
    });
  }
}
