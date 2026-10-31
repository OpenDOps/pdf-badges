import { Injectable, signal } from '@angular/core';

@Injectable({ providedIn: 'root' })
export class WorkstationPrinterService {
  readonly selected = signal<{ dispatch_mode?: string; agent_kind?: string; dpi: number } | null>(null);
}
