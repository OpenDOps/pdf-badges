import { Injectable } from '@angular/core';
import { of } from 'rxjs';

@Injectable({ providedIn: 'root' })
export class PublicApiService {
  getFormlySchema(_ticketTypeId: number | string) {
    return of(null);
  }
}
