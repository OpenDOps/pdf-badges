import { Injectable, signal } from '@angular/core';

@Injectable({ providedIn: 'root' })
export class ToastService {
  readonly message = signal<string | null>(null);

  error(message: string) {
    this.message.set(message);
  }

  success(message: string) {
    this.message.set(message);
  }
}
