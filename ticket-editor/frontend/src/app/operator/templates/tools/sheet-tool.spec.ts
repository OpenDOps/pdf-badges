import type { PageDocument, PageEntry } from '../page-view';
import { entryFromTarget } from './sheet-tool';

function page(contents: PageEntry[]): PageDocument {
  return {
    page_size: { width: 100, height: 100, units: 'mm' },
    resources: { fonts: { text: { font_family: 'Liberation Serif' } } },
    contents,
  };
}

describe('entryFromTarget', () => {
  it('resolves the live page entry by data-yaml-id under the target', () => {
    const entry: PageEntry = {
      id: 'p1-t1',
      posX: 0,
      posY: 0,
      width: 10,
      height: 5,
      preentered: 'Hi',
      text: { font: 'text', font_size: 4, leading: 0, line_height: 5 },
    };
    const sheet = page([entry]);
    const box = document.createElement('div');
    box.setAttribute('data-yaml-id', 'p1-t1');
    const child = document.createElement('span');
    box.appendChild(child);
    document.body.appendChild(box);

    expect(entryFromTarget(sheet, child)).toBe(entry);
    expect(entryFromTarget(sheet, box)).toBe(entry);
    expect(entryFromTarget(sheet, document.body)).toBeNull();
    expect(entryFromTarget(null, child)).toBeNull();
    expect(entryFromTarget(sheet, null)).toBeNull();

    document.body.replaceChildren();
  });

  it('returns null when the id is not on the page', () => {
    const sheet = page([]);
    const box = document.createElement('div');
    box.setAttribute('data-yaml-id', 'missing');
    expect(entryFromTarget(sheet, box)).toBeNull();
  });
});
