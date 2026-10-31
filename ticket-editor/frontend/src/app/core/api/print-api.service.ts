import { HttpClient } from '@angular/common/http';
import { Injectable, inject } from '@angular/core';
import { Observable } from 'rxjs';

import { environment } from '../../../environments/environment';
import {
  DemoContext,
  PageFormatsCatalog,
  PrintTemplateCreateRequest,
  PrintTemplateDetail,
  PrintTemplateWrite,
} from '../models/print.models';

type TemplateId = number | string;

@Injectable({ providedIn: 'root' })
export class PrintApiService {
  private readonly http = inject(HttpClient);
  private readonly base = environment.apiBaseUrl;

  formats(): Observable<PageFormatsCatalog> {
    return this.http.get<PageFormatsCatalog>(`${this.base}/print_template/formats`);
  }

  demoContext(ticketTypeId: number): Observable<DemoContext> {
    return this.http.get<DemoContext>(`${this.base}/print_template/demo_context`, {
      params: { ticket_type: ticketTypeId },
    });
  }

  createTemplate(payload: PrintTemplateCreateRequest): Observable<PrintTemplateDetail> {
    return this.http.post<PrintTemplateDetail>(`${this.base}/print_template/create`, payload);
  }

  getTemplate(id: TemplateId): Observable<PrintTemplateDetail> {
    return this.http.get<PrintTemplateDetail>(`${this.base}/print_template/${id}`);
  }

  updateTemplate(id: TemplateId, payload: PrintTemplateWrite): Observable<PrintTemplateDetail> {
    return this.http.put<PrintTemplateDetail>(`${this.base}/print_template/${id}`, payload);
  }

  uploadAsset(id: TemplateId, name: string, file: File): Observable<{ name: string; path: string; manifest: Record<string, string> }> {
    const form = new FormData();
    form.append('name', name);
    form.append('file', file, file.name);
    return this.http.post<{ name: string; path: string; manifest: Record<string, string> }>(
      `${this.base}/print_template/${id}/assets`,
      form,
    );
  }

  deleteAsset(id: TemplateId, name: string): Observable<{ manifest: Record<string, string> }> {
    return this.http.delete<{ manifest: Record<string, string> }>(
      `${this.base}/print_template/${id}/assets/${encodeURIComponent(name)}`,
    );
  }

  getAsset(id: TemplateId, name: string): Observable<Blob> {
    return this.http.get(`${this.base}/print_template/${id}/assets/${encodeURIComponent(name)}`, {
      responseType: 'blob',
    });
  }
}
