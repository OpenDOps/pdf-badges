import { breaksToLines, ceilHalfMm, createTextStyleCache, linesToBreaks, textFrameMm, type TextStyleMemory } from './text-style';

describe('text style cache', () => {
  function storage() {
    const bag = new Map<string, string>();
    return {
      getItem: (key: string) => bag.get(key) ?? null,
      setItem: (key: string, value: string) => {
        bag.set(key, value);
      },
      raw: () => bag.get('ticket-editor:text-style') ?? '',
    };
  }

  it('starts at 12 pt Liberation and reuses the last face', () => {
    const store = storage();
    const cache = createTextStyleCache(store);
    expect(cache.read()).toEqual({ font: 'text', fontSizePt: 12, bold: false, italic: false, underline: false, foreground: '#000000', background: '#FFFFFF' });

    const picked: TextStyleMemory = { font: 'display', fontSizePt: 18, bold: true, italic: false, underline: true, foreground: '#FF6A00', background: '#112233' };
    cache.remember(picked);
    expect(cache.read()).toEqual(picked);
    expect(JSON.parse(store.raw())).toEqual(picked);

    const again = createTextStyleCache(store);
    expect(again.read()).toEqual(picked);
  });

  it('ignores a broken store', () => {
    const cache = createTextStyleCache({
      getItem: () => '{',
      setItem: () => undefined,
    });
    expect(cache.read().fontSizePt).toBe(12);
  });
});

describe('glyph frame', () => {
  const advance = (line: string) => line.length * 2;

  it('sizes one line from glyph advances and stacks a break', () => {
    expect(textFrameMm('Текст', 4.233, 5.08, 0, 0, advance)).toEqual({ width: 10, height: 5.08 });
    expect(textFrameMm('Текст\nТекст', 4.233, 5.08, 0, 0, advance)).toEqual({ width: 10, height: 10.16 });
    expect(ceilHalfMm(5.08)).toBe(5.5);
    expect(ceilHalfMm(10)).toBe(10);
  });

  it('adds letter spacing between glyphs and keeps an empty line one em wide', () => {
    expect(textFrameMm('Ab', 4, 5, 0, 0.5, advance).width).toBe(4.5);
    expect(textFrameMm('', 4.233, 5.08, 0, 0, () => 0)).toEqual({ width: 4.233, height: 5.08 });
  });

  it('stacks a line height and the gap between lines', () => {
    expect(textFrameMm('Раз\nДва\nТри', 4.233, 5.08, 1.5, 0, advance).height).toBeCloseTo(18.24, 5);
  });
});

describe('line breaks', () => {
  it('turns Enter into br and br back into a line', () => {
    expect(linesToBreaks('Раз\nДва')).toBe('Раз<br>Два');
    expect(linesToBreaks('Раз\r\nДва\rТри')).toBe('Раз<br>Два<br>Три');
    expect(breaksToLines('Раз<br>Два<br/>Три<br />')).toBe('Раз\nДва\nТри\n');
    expect(linesToBreaks(breaksToLines('{{name}}<br>Текст'))).toBe('{{name}}<br>Текст');
  });
});
