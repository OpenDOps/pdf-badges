import { demoContext } from '../../../../../src/demo-context';
import { environment } from '../../../environments/environment';
import { environment as editorEnvironment } from '../../../environments/environment.editor';
import {
  PageDocument,
  appendCode,
  appendMustache,
  appendPaletteEntry,
  appendRect,
  appendTextAt,
  applyEntryColor,
  applyPageJson,
  flatMustache,
  formatPageJson,
  growEntryToText,
  operatorStyleSectors,
  entryToHtml,
  pageToHtml,
  ptToMm,
  removeEntry,
  syncEntryFromCanvas,
  yamlStyleSectors,
} from './page-view';
import { normalizeRect, placedCodeRect } from './draw-tool';
import type { TextStyleMemory } from './text-style';

function samplePage(): PageDocument {
  return {
    page_size: { width: 100, height: 100, units: 'mm' },
    resources: {
      images: { logo: { name: 'logo', extracted_path: 'files/logo.png' } },
      fonts: { text: { font_family: 'Liberation Serif' } },
    },
    contents: [
      {
        id: 'p1-t1',
        posX: 10,
        posY: 20,
        width: 80,
        height: 12,
        template: true,
        preentered: '{{field_1|upper}}',
        text: { font: 'text', font_size: 6.35, leading: 0, line_height: 7.62 },
      },
      {
        id: 'p1-b1',
        posX: 10,
        posY: 40,
        width: 25,
        height: 25,
        template: true,
        barcode: { type: 'qr', preentered: '{{code}}' },
      },
      {
        id: 'p1-i1',
        posX: 60,
        posY: 40,
        width: 30,
        height: 20,
        image: { resource_name: 'logo' },
      },
    ],
  };
}

describe('pageToHtml', () => {
  it('projects a variable, a QR, and a logo in millimetres', () => {
    const html = pageToHtml(samplePage());

    expect(html).toContain('class="tpl-var"');
    expect(html).toContain('data-var="form_data.field_1"');
    expect(html).toContain('data-case="upper"');
    expect(html).toContain('data-yaml-id="p1-t1"');
    expect(html).toContain('left:10mm');
    expect(html).toContain('top:20mm');
    expect(html).toContain('width:80mm');
    expect(html).toContain('height:12mm');
    expect(html).toContain('{{field_1|upper}}');

    expect(html).toContain('class="tpl-code"');
    expect(html).toContain('data-code="qrcode"');
    expect(html).toContain('data-yaml-id="p1-b1"');
    expect(html).toContain(`src="${demoContext.codes.qrcode_svg}"`);
    const qrSvg = atob(demoContext.codes.qrcode_svg.split(',')[1]);
    expect(qrSvg.match(/shape-rendering=/g)).toEqual(['shape-rendering=']);
    expect(new DOMParser().parseFromString(qrSvg, 'image/svg+xml').querySelector('parsererror')).toBeNull();

    expect(html).toContain('class="tpl-file"');
    expect(html).toContain('data-file="logo"');
    expect(html).toContain('data-yaml-id="p1-i1"');
    expect(html).toContain('left:60mm');
    expect(html).toContain('top:40mm');
  });

  it('entryToHtml matches the fragment pageToHtml embeds for that id', () => {
    const page = samplePage();
    const entry = page.contents[0];
    const fragment = entryToHtml(entry, page);
    expect(fragment).toContain('data-yaml-id="p1-t1"');
    expect(pageToHtml(page)).toContain(fragment);
  });

  it('keeps a literal as text and omits identity frame values', () => {
    const page = samplePage();
    page.contents = [
      {
        id: 'p1-t1',
        posX: 4,
        posY: 6,
        width: 40,
        height: 8,
        preentered: 'Hello',
        rotation: 0,
        opacity: 1,
        scale_x: 1,
        scale_y: 1,
        skew_x: 0,
        skew_y: 0,
        background: '#FF6A00',
        text: { font: 'text', font_size: 6.35, leading: 0, line_height: 7.62, letter_spacing: 0 },
      },
    ];
    const html = pageToHtml(page);

    expect(html).toContain('class="tpl-text"');
    expect(html).toContain('>Hello</div>');
    expect(html).toContain('background-color:#FF6A00');
    expect(html).not.toContain('data-rotation');
    expect(html).not.toContain('opacity:');
    expect(html).not.toContain('data-scale-x');
    expect(html).not.toContain('data-skew-x');
    expect(html).not.toContain('letter-spacing');
    expect(html).not.toContain('@font-face');
    expect(html).toContain('data-text="1"');
  });

  it('hides a box whose ifSt is missing or false and shows field_6', () => {
    const page = samplePage();
    page.contents[0].ifSt = 'field_6';
    expect(pageToHtml(page)).toContain('data-if="field_6"');
    expect(pageToHtml(page)).not.toContain('display:none');

    page.contents[0].ifSt = 'field_missing';
    expect(pageToHtml(page)).toContain('display:none');
  });

  it('draws two mustaches as the demo names and keeps the source on the attribute', () => {
    const page = samplePage();
    page.contents = [
      {
        id: 'p1-t1',
        posX: 5,
        posY: 5,
        width: 40,
        height: 10,
        template: true,
        preentered: '{{field_1|upper}}<br>{{field_2}}',
        content: null,
        text: { font: 'text', font_size: 4.233, leading: 0, line_height: 5.08 },
      },
    ];
    const html = pageToHtml(page);
    expect(html).toContain('data-preentered="&#123;&#123;field_1|upper&#125;&#125;&lt;br&gt;&#123;&#123;field_2&#125;&#125;"');
    expect(html).toContain('ПЕТРОВА');
    expect(html).toContain('Анна');
    expect(html).toContain('class="tpl-text"');
  });
});

describe('page edits', () => {
  function emptyPage(): PageDocument {
    return {
      page_size: { width: 100, height: 100, units: 'mm', bleeds: { top: 0, right: 0, bottom: 0, left: 0 } },
      resources: { fonts: { text: { font_family: 'Liberation Serif' } }, images: {} },
      contents: [],
    };
  }

  it('places text at the snapped click, sized from the font, then a second one', () => {
    const page = emptyPage();
    const stored = structuredClone(page);
    const style: TextStyleMemory = { font: 'text', fontSizePt: 12, bold: false, italic: false, underline: false, foreground: '#000000', background: '#FFFFFF' };
    const measure = (line: string) => line.length * ptToMm(style.fontSizePt) * 0.5;
    const first = appendTextAt(page, { x: 12.2, y: 7.4 }, { style, measureLine: measure });
    expect(first).toMatchObject({
      id: 'p1-t1',
      posX: 12,
      posY: 2,
      width: 11,
      height: 5.5,
      preentered: 'Текст',
      content: null,
      alignment: { horizontal: 'left', vertical: 'top' },
      padding: { top: 0, right: 0, bottom: 0, left: 0 },
      text: { font: 'text', font_size: 4.233, leading: 0, line_height: 5.08 },
    });
    const second = appendTextAt(page, { x: 30.1, y: 20.2 }, { style, measureLine: measure });
    expect(second.id).toBe('p1-t2');
    expect(second.posX).toBe(30);
    expect(second.posY).toBe(14.5);
    expect(second.width).toBe(11);
    expect(second.height).toBe(5.5);
    expect(page.contents.map((entry) => entry.id)).toEqual(['p1-t1', 'p1-t2']);
    expect(stored.contents).toEqual([]);
  });

  it('uses a larger remembered face, and grows only when autoscale is off', () => {
    const page = emptyPage();
    const large: TextStyleMemory = { font: 'display', fontSizePt: 24, bold: true, italic: false, underline: false, foreground: '#000000', background: '#FFFFFF' };
    const wide = (line: string) => line.length * ptToMm(24) * 0.5;
    const sized = appendTextAt(page, { x: 1, y: 1 }, { style: large, measureLine: wide });
    expect(sized.text).toMatchObject({ font: 'text', font_size: 8.467, line_height: 10.16, font_style: { bold: true } });
    expect(sized.width).toBe(21.5);
    expect(sized.height).toBe(10.5);

    const style: TextStyleMemory = { font: 'text', fontSizePt: 12, bold: false, italic: false, underline: false, foreground: '#000000', background: '#FFFFFF' };
    const measure = (line: string) => line.length * ptToMm(12) * 0.5;
    const entry = appendTextAt(page, { x: 1, y: 1 }, { style, measureLine: measure });
    entry.preentered = 'Текст Текст';
    expect(growEntryToText(entry, measure)).toBe(true);
    expect(entry.width).toBe(23.5);

    entry.auto_scale = true;
    entry.preentered = 'Текст Текст Текст';
    expect(growEntryToText(entry, measure)).toBe(false);
    expect(entry.width).toBe(23.5);

    entry.auto_scale = false;
    entry.preentered = 'Текст<br>ещё';
    expect(growEntryToText(entry, measure)).toBe(true);
    expect(entry.height).toBe(10.5);

    entry.text!.leading = 1.5;
    entry.preentered = 'Раз<br>Два<br>Три';
    expect(growEntryToText(entry, measure)).toBe(true);
    expect(entry.height).toBe(18.5);
  });

  it('appends one empty text box for a rectangle drag, and nothing under 2 mm', () => {
    const page = emptyPage();
    const style: TextStyleMemory = { font: 'text', fontSizePt: 12, bold: false, italic: false, underline: false, foreground: '#000000', background: '#FFFFFF' };
    expect(normalizeRect({ x: 5, y: 5 }, { x: 6, y: 15 })).toBeNull();
    expect(page.contents).toEqual([]);

    const drawn = normalizeRect({ x: 5, y: 5 }, { x: 25, y: 15 });
    const entry = appendRect(page, drawn!, style);
    expect(entry).toMatchObject({
      id: 'p1-t1',
      posX: 5,
      posY: 5,
      width: 20,
      height: 10,
      preentered: '',
      content: null,
      background: '#FFFFFF',
      border: { width: 0.2, color: '#000000' },
      alignment: { horizontal: 'left', vertical: 'top' },
      padding: { top: 0, right: 0, bottom: 0, left: 0 },
      text: { font: 'text', font_size: 4.233, leading: 0, line_height: 5.08 },
    });
    expect(entry.template).toBeUndefined();
    expect(entry.preentered).not.toContain('{{');

    const reversed = normalizeRect({ x: 25, y: 15 }, { x: 5, y: 5 });
    const second = appendRect(page, reversed!, {
      font: 'missing',
      fontSizePt: 24,
      bold: true,
      italic: false,
      underline: false,
      foreground: '#000000',
      background: '#FFFFFF',
    });
    expect(second.id).toBe('p1-t2');
    expect(second.posX).toBe(5);
    expect(second.posY).toBe(5);
    expect(second.width).toBe(20);
    expect(second.height).toBe(10);
    expect(second.text).toMatchObject({ font: 'text', font_size: 8.467, line_height: 10.16, font_style: { bold: true } });
    expect(page.contents).toHaveLength(2);
  });

  it('appends a QR or an EAN-13 for a corner drag, and a square only when QR holds Shift', () => {
    const page = emptyPage();
    expect(normalizeRect({ x: 0, y: 0 }, { x: 1, y: 12 })).toBeNull();
    expect(page.contents).toEqual([]);

    const drag = normalizeRect({ x: 0, y: 0 }, { x: 30, y: 12 })!;
    const qr = appendCode(page, placedCodeRect(drag, 'qr', false), 'qr');
    expect(qr).toMatchObject({
      id: 'p1-b1',
      posX: 0,
      posY: 0,
      width: 30,
      height: 12,
      template: true,
      content: null,
      barcode: { type: 'qr', preentered: '{{code}}' },
    });
    expect(qr.text).toBeUndefined();
    expect(qr.image).toBeUndefined();
    expect(qr.photo).toBeUndefined();
    const qrHtml = pageToHtml(page);
    expect(qrHtml).toContain('data-code="qrcode"');
    expect(qrHtml).toContain(`src="${demoContext.codes.qrcode_svg}"`);

    const squared = appendCode(page, placedCodeRect(drag, 'qr', true), 'qr');
    expect(squared).toMatchObject({ id: 'p1-b2', posX: 0, posY: 0, width: 30, height: 30, barcode: { type: 'qr' } });

    const fromAnchor = normalizeRect({ x: 10, y: 4 }, { x: 40, y: 16 })!;
    const anchored = placedCodeRect(fromAnchor, 'qr', true);
    expect(anchored).toMatchObject({ posX: 10, posY: 4, width: 30, height: 30, anchorX: 10, anchorY: 4 });

    const barcode = appendCode(page, placedCodeRect(drag, 'ean13', true), 'ean13');
    expect(barcode).toMatchObject({
      id: 'p1-b3',
      posX: 0,
      posY: 0,
      width: 30,
      height: 12,
      template: true,
      content: null,
      barcode: { type: 'ean13', preentered: '{{code}}' },
    });
    const barcodeHtml = pageToHtml(page);
    expect(barcodeHtml).toContain('data-code="barcode"');
    expect(barcodeHtml).toContain(`src="${demoContext.codes.barcode_svg}"`);
    expect(page.contents).toHaveLength(3);

    expect(removeEntry(page, 'p1-b2')).toBe(true);
    expect(page.contents.map((entry) => entry.id)).toEqual(['p1-b1', 'p1-b3']);
    expect(removeEntry(page, 'p1-b2')).toBe(false);
    const again = appendCode(page, drag, 'ean13');
    expect(again.id).toBe('p1-b4');
    expect(pageToHtml(page)).not.toContain('data-yaml-id="p1-b2"');
  });

  it('paints foreground onto text and background onto a rectangle', () => {
    const page = emptyPage();
    const face: TextStyleMemory = { font: 'text', fontSizePt: 12, bold: false, italic: false, underline: false, foreground: '#FF6A00', background: '#112233' };
    const measure = (line: string) => line.length * ptToMm(12) * 0.5;
    const text = appendTextAt(page, { x: 1, y: 1 }, { style: face, measureLine: measure });
    expect(text.text?.color).toBe('#FF6A00');
    const rect = appendRect(page, { posX: 5, posY: 5, width: 20, height: 10 }, face);
    expect(rect.background).toBe('#112233');
    expect(rect.text?.color).toBe('#FF6A00');
    expect(rect.border).toEqual({ width: 0.2, color: '#000000' });

    expect(applyEntryColor(text, 'foreground', 'orange')).toBe(false);
    expect(text.text?.color).toBe('#FF6A00');
    expect(applyEntryColor(text, 'foreground', '#000000')).toBe(true);
    expect(text.text?.color).toBeUndefined();
    expect(applyEntryColor(rect, 'background', '#ABCDEF')).toBe(true);
    expect(rect.background).toBe('#ABCDEF');

    const image = appendPaletteEntry(page, { pageKind: 'file', fileName: 'mark.png' });
    expect(applyEntryColor(image, 'foreground', '#FF6A00')).toBe(false);
    expect(applyEntryColor(image, 'background', '#00FF00')).toBe(true);
    expect(image.background).toBe('#00FF00');
  });

  it('appends a variable, then a second mustache in the selected box', () => {
    const page = emptyPage();
    const entry = appendPaletteEntry(page, { pageKind: 'var', editorPath: 'form_data.field_1' });
    expect(entry.id).toBe('p1-t1');
    expect(entry.preentered).toBe('{{field_1}}');
    expect(entry.template).toBe(true);
    expect(entry.content).toBeNull();
    expect(entry.alignment?.vertical).toBe('top');
    expect(entry.text?.font).toBe('text');
    expect(entry.text?.leading).toBe(0);
    expect(entry.auto_scale).toBeUndefined();
    expect(entry.posX).toBe(5);

    appendMustache(entry, 'field_2');
    expect(entry.preentered).toBe('{{field_1}}<br>{{field_2}}');
    expect(page.contents.length).toBe(1);
  });

  it('starts a QR square and keeps an existing pipe when the case changes', () => {
    const page = emptyPage();
    const code = appendPaletteEntry(page, { pageKind: 'code', codeKind: 'qrcode', posX: 12, posY: 8 });
    expect(code.id).toBe('p1-b1');
    expect(code.width).toBe(code.height);
    expect(code.barcode?.type).toBe('qr');

    const text = appendPaletteEntry(page, { pageKind: 'var', editorPath: 'field_1' });
    syncEntryFromCanvas(text, {
      style: { left: '12mm', top: '8mm', width: '40mm', height: '10mm', 'font-size': '18pt', 'line-height': '1.2' },
      attrs: {
        'data-var': 'form_data.field_1',
        'data-case': 'upper',
        'data-valign': 'middle',
        'data-auto-scale': 'true',
        'data-leading': '0',
        'data-font': 'text',
        'data-if': 'field_6',
      },
    });
    expect(text.preentered).toBe('{{field_1|upper}}');
    expect(text.posX).toBe(12);
    expect(text.alignment?.vertical).toBe('middle');
    expect(text.auto_scale).toBe(true);
    expect(text.ifSt).toBe('field_6');
    expect(text.text?.font).toBe('text');
    expect(text.text?.font_size).toBeCloseTo(6.35, 2);
    expect(flatMustache('ticket.ticket_code.code')).toBe('{{code}}');
  });

  it('leaves a span byte for byte when the box only moves', () => {
    const page = emptyPage();
    const source = 'Hello <span color="#112233">{{field_1}}</span>';
    page.contents.push({
      id: 'p1-t1',
      posX: 10,
      posY: 20,
      width: 80,
      height: 12,
      template: true,
      preentered: source,
      content: null,
      text: { font: 'text', font_size: 6.35, leading: 0, line_height: 7.62 },
    });
    syncEntryFromCanvas(
      page.contents[0],
      {
        style: { left: '18mm', top: '20mm', width: '80mm', height: '12mm' },
        attrs: { 'data-font': 'text', 'data-valign': 'top', 'data-leading': '0' },
      },
      'geometry',
    );
    expect(page.contents[0].preentered).toBe(source);
    expect(page.contents[0].posX).toBe(18);
  });

  it('rejects a page the writer would reject and keeps the caller object', () => {
    const page = emptyPage();
    const bad = formatPageJson(page).replace('"contents": []', '"contents": [{"id":"p1-t1","posX":1,"posY":1,"width":10,"height":10,"preentered":"<div>x</div>","text":{"font":"text","font_size":4,"leading":0,"line_height":5}}]');
    const applied = applyPageJson(bad);
    expect(applied.ok).toBe(false);
    if (!applied.ok) expect(applied.detail).toContain('unknown tag');
    expect(page.contents).toEqual([]);

    const orange = formatPageJson({
      ...page,
      contents: [
        {
          id: 'p1-t1',
          posX: 1,
          posY: 1,
          width: 10,
          height: 10,
          preentered: 'Hi',
          text: { font: 'text', font_size: 4, leading: 0, line_height: 5 },
          background: 'orange',
        },
      ],
    });
    expect(applyPageJson(orange).ok).toBe(false);
  });

  it('offers the page fields on the yaml route and keeps percent sizes on the operator route', () => {
    const yaml = yamlStyleSectors().flatMap((sector) => sector.properties);
    const names = yaml.map((property) => property.property);
    expect(names).toEqual(expect.arrayContaining([
      'background-color',
      'border-width',
      'border-radius',
      'opacity',
      'letter-spacing',
      '--scale-x',
      '--scale-y',
      '--skew-x',
      '--skew-y',
      '--rotation',
    ]));
    expect(names).not.toContain('white-space');
    expect(names).not.toContain('text-overflow');
    expect(names).not.toContain('overflow');
    const width = yaml.find((property) => property.property === 'width');
    expect(width?.units).toEqual(['mm']);
    const rotation = yaml.find((property) => property.property === '--rotation');
    expect(rotation?.options?.map((option) => option.id)).toEqual(['0', '90']);

    const operator = operatorStyleSectors().flatMap((sector) => sector.properties);
    expect(operator.find((property) => property.property === 'width')?.units).toContain('%');
    expect(operator.map((property) => property.property)).toEqual(expect.arrayContaining(['background-color', 'border']));
    expect(editorEnvironment.yamlPage).toBe(true);
    expect(environment.yamlPage).toBe(true);
  });
});
