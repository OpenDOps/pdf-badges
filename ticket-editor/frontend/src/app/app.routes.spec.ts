import { provideHttpClient } from '@angular/common/http';
import { HttpTestingController, provideHttpClientTesting } from '@angular/common/http/testing';
import { TestBed } from '@angular/core/testing';
import { provideRouter, Router } from '@angular/router';
import { RouterTestingHarness } from '@angular/router/testing';
import { routes } from './app.routes';

function flushList() {
  const http = TestBed.inject(HttpTestingController);
  for (const request of http.match('/api/print_template/list')) request.flush([]);
}

function flushEditor(id = '1') {
  const http = TestBed.inject(HttpTestingController);
  const requests = http.match(`http://127.0.0.1:4310/api/print_template/${id}`);
  for (const request of requests) {
    request.flush({
      id,
      name: 'Badge',
      source: 'local',
      is_default: true,
      html: '<div></div>',
      editor_state: null,
      page: null,
      manifest: {},
      updated_at: '',
      page_width_mm: 100,
      page_height_mm: 100,
      margin_top_mm: 0,
      margin_right_mm: 0,
      margin_bottom_mm: 0,
      margin_left_mm: 0,
    });
  }
}

describe('routes', () => {
  it('lists the editor paths in order', () => {
    expect(routes.map((route) => route.path)).toEqual(['editor/list', 'editor', 'editor/:id', '**']);
    expect(routes[1].pathMatch).toBe('full');
    expect(routes.some((route) => route.path?.includes('operator'))).toBe(false);
    const editor = routes.find((route) => route.path === 'editor/:id');
    expect(editor?.children?.[0].data).toEqual({ localCatalog: true });
  });

  it('opens the sheet and follows the redirects', async () => {
    TestBed.configureTestingModule({
      providers: [provideRouter(routes), provideHttpClient(), provideHttpClientTesting()],
    });
    const harness = await RouterTestingHarness.create();
    const router = TestBed.inject(Router);

    await harness.navigateByUrl('/editor/list');
    flushList();
    expect(harness.routeNativeElement?.textContent).toContain('Макеты');

    await harness.navigateByUrl('/editor/1');
    flushEditor();
    harness.detectChanges();
    expect(harness.routeNativeElement?.querySelector('#template-name')).toBeTruthy();
    expect(router.url).toBe('/editor/1');

    await harness.navigateByUrl('/editor');
    flushEditor();
    expect(router.url).toBe('/editor/1');

    await harness.navigateByUrl('/nope');
    flushList();
    expect(router.url).toBe('/editor/list');

    await harness.navigateByUrl('/operator/templates/1');
    flushList();
    expect(router.url).toBe('/editor/list');
  });
});
