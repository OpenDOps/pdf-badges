import { DemoContext } from '../../core/models/print.models';
import {
  buildPalette,
  codeBlockHtml,
  codeWidthMm,
  snapCodeWidthMm,
  demoValueFor,
  joinDocument,
  splitDocument,
  variableBlockHtml,
  variableExpression,
  visualModeBlocker,
} from './template-snippets';

const demo: DemoContext = {
  fields: [
    { key: 'field_1', label: 'Фамилия', kind: 'text', section: '', sample: 'Петрова' },
    { key: 'field_2', label: 'Имя', kind: 'text', section: '', sample: 'Анна' },
    { key: 'field_3', label: 'Интересы', kind: 'multichoice', section: 'Анкета', sample: ['a', 'b'] },
  ],
  ticket: { number: 'A-00042', code: '2000000000008', ticket_type_name: 'Standard' },
  form_data: { field_1: 'Петрова', field_2: 'Анна', field_3: ['a', 'b'] },
  has_vcard: false,
  codes: { barcode: '', qrcode: '', vcard_qrcode: '', barcode_svg: '', qrcode_svg: '', vcard_qrcode_svg: '' },
  geometry: { ean13_modules: 117, qr_modules: 29 },
};

describe('template snippets', () => {
  it('builds Django expressions with only builtin filters', () => {
    expect(variableExpression('form_data.field_1')).toBe('{{ form_data.field_1 }}');
    expect(variableExpression('form_data.field_1', { case: 'upper', default: '—' })).toBe(
      '{{ form_data.field_1|default:"—"|upper }}'
    );
    expect(variableExpression('form_data.field_3', {}, true)).toBe('{{ form_data.field_3|join:", " }}');
  });

  it('marks variable blocks with data-var so the visual mode recognises them again', () => {
    const html = variableBlockHtml(
      { expr: 'form_data.field_1', label: 'Фамилия', kind: 'text', sample: 'Петрова', isList: false },
      { prefix: 'Фамилия: ' }
    );
    expect(html).toContain('data-var="form_data.field_1"');
    expect(html).toContain('data-prefix="Фамилия: "');
    expect(html).toContain('Фамилия: {{ form_data.field_1 }}');
  });

  it('sizes codes in whole printer dots per module (раздел 6.4 ТЗ)', () => {
    // EAN-13: 117 modules × 3 dots × (25.4 / 203) mm — exact dpi, not the
    // "0.125 mm per dot" approximation (203 dpi is 7.99 dots/mm, not 8)
    expect(codeWidthMm('barcode', 3, 203, 29)).toBeCloseTo((117 * 3 * 25.4) / 203, 2);
    // QR version 1 + border: 29 modules × 4 dots
    expect(codeWidthMm('qrcode', 4, 203, 29)).toBeCloseTo((29 * 4 * 25.4) / 203, 2);
    const block = codeBlockHtml('barcode', { widthMm: 43.875, heightMm: 15 });
    expect(block).toContain('src="{{ barcode_svg }}"');
    expect(block).toContain('data-code="barcode"');
    expect(codeBlockHtml('qrcode', { widthMm: 14.5, vector: false })).toContain('src="{{ qrcode }}"');
  });

  it('snaps a freely resized code to the nearest whole-dot module width', () => {
    // 50 mm is between 3 dots (43.9) and 4 dots (58.6) → 3 dots is nearer
    expect(snapCodeWidthMm('barcode', 50, 203, 29)).toEqual({ widthMm: codeWidthMm('barcode', 3, 203, 29), dots: 3 });
    // below the smallest readable module → clamps to 2 dots
    expect(snapCodeWidthMm('barcode', 5, 203, 29).dots).toBe(2);
    // far beyond the largest → clamps to the last option
    expect(snapCodeWidthMm('qrcode', 90, 203, 29).dots).toBe(6);
    expect(snapCodeWidthMm('qrcode', 18, 203, 29).dots).toBe(5);
  });

  it('groups palette items by survey section and adds composites when Фамилия/Имя are found', () => {
    const schema = [
      { key: 'field_1', type: 'input', props: { label: 'Фамилия' } },
      { key: 'field_2', type: 'input', props: { label: 'Имя' } },
    ];
    const groups = buildPalette(demo, schema);
    expect(groups.map((g) => g.title)).toEqual(['Анкета', 'Анкета — Анкета', 'Анкета — составные', 'Билет']);
    const composite = groups.find((g) => g.title === 'Анкета — составные')!;
    expect(composite.items[0].sample).toBe('Петрова Анна');
    expect(demoValueFor(composite.items[0].expr, demo)).toBe('Петрова Анна');
    const list = groups[1].items[0];
    expect(list.isList).toBe(true);
  });

  it('resolves demo values for ticket and survey expressions', () => {
    expect(demoValueFor('ticket.number', demo)).toBe('A-00042');
    expect(demoValueFor('form_data.field_3', demo)).toBe('a, b');
    expect(demoValueFor('form_data.field_404', demo)).toBe('');
  });

  it('blocks the visual mode on template logic, scripts and external links', () => {
    expect(visualModeBlocker('<div>{{ x }}</div>')).toBeNull();
    expect(visualModeBlocker('{% if x %}a{% endif %}')).toContain('логика');
    expect(visualModeBlocker('<script>1</script>')).toContain('script');
    expect(visualModeBlocker('<link rel="stylesheet" href="x.css">')).toContain('link');
  });

  it('splits and re-joins a document around its <style> blocks', () => {
    const { css, body } = splitDocument('<html><head><style>b{color:red}</style></head><body><p>x</p></body></html>');
    expect(css).toBe('b{color:red}');
    expect(body).toBe('<p>x</p>');
    const doc = joinDocument(css, body);
    expect(doc).toContain('<style>\nb{color:red}\n</style>');
    expect(doc).toContain('<body>\n<p>x</p>\n</body>');
  });
});
