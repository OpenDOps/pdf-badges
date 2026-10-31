import { provideHttpClient } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { TestBed } from '@angular/core/testing';
import { provideRouter } from '@angular/router';
import { RouterTestingHarness } from '@angular/router/testing';

import { routes } from '../../app.routes';

describe('template editor', () => {
  it('has no variable palette and no file upload', async () => {
    TestBed.configureTestingModule({
      providers: [provideRouter(routes), provideHttpClient(), provideHttpClientTesting()],
    });
    const harness = await RouterTestingHarness.create();
    await harness.navigateByUrl('/editor/1');
    TestBed.inject(HttpTestingController).expectOne('http://127.0.0.1:4310/api/print_template/1').flush({
      id: 1,
      ticket_type: 1,
      ticket_type_name: '',
      name: 'Badge',
      source: 'local',
      is_default: true,
      updated_at: '',
      updated_by_name: '',
      page_format: 'custom',
      page_width_mm: 100,
      page_height_mm: 100,
      orientation: 'portrait',
      margin_top_mm: 0,
      margin_right_mm: 0,
      margin_bottom_mm: 0,
      margin_left_mm: 0,
      html: '',
      editor_state: null,
      manifest: {},
      created_at: '',
    });
    await harness.fixture.whenStable();
    harness.detectChanges();

    const root = harness.fixture.nativeElement as HTMLElement;
    expect(root.querySelector('app-variable-palette')).toBeNull();
    expect(root.textContent).not.toContain('Загрузить файл');
  });
});
