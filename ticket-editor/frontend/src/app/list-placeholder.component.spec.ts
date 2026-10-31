import { provideHttpClient } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { RouterTestingHarness } from '@angular/router/testing';
import { routes } from './app.routes';

describe('template list', () => {
  it('shows each template name, size, and edit link', async () => {
    TestBed.configureTestingModule({
      providers: [provideRouter(routes), provideHttpClient(), provideHttpClientTesting()],
    });
    const harness = await RouterTestingHarness.create();
    await harness.navigateByUrl('/editor/list');
    TestBed.inject(HttpTestingController).expectOne('/api/print_template/list').flush([
      { id: 1, name: 'Badge', updated_at: '', page_width_mm: 100, page_height_mm: 100 },
      {
        id: 'pir',
        name: 'ticket_visitor_pirexpo2026',
        updated_at: '2026-01-02T00:00:00.000Z',
        page_width_mm: 210,
        page_height_mm: 297,
      },
    ]);
    await harness.fixture.whenStable();
    harness.detectChanges();

    const text = harness.routeNativeElement?.textContent ?? '';
    expect(text).toContain('Макеты');
    expect(text).toContain('Badge');
    expect(text).toContain('100×100 мм');
    expect(text).toContain('ticket_visitor_pirexpo2026');
    expect(text).toContain('210×297 мм');
    expect(text).toContain('2026-01-02T00:00:00.000Z');
    expect(text).toContain('Редактировать');
    const links = harness.routeNativeElement?.querySelectorAll('a') ?? [];
    expect(Array.from(links).some((link) => link.getAttribute('href') === '/editor/pir')).toBe(true);
  });
});
