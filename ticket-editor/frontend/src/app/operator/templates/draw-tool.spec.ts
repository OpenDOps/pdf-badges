import { PX_PER_MM, boxesInside, lockSquare, normalizeRect } from './draw-tool';

describe('normalizeRect', () => {
  it('stores the lesser corner and the positive sides', () => {
    expect(normalizeRect({ x: 10, y: 8 }, { x: 4, y: 20 })).toEqual({
      posX: 4,
      posY: 8,
      width: 6,
      height: 12,
      anchorX: 10,
      anchorY: 8,
    });
  });

  it('returns null when a side is under 2 mm', () => {
    expect(normalizeRect({ x: 0, y: 0 }, { x: 1, y: 10 })).toBeNull();
  });
});

describe('lockSquare', () => {
  it('grows a 10×4 drag into a 10×10 square from the anchor', () => {
    const rect = normalizeRect({ x: 0, y: 0 }, { x: 10, y: 4 });
    expect(rect).not.toBeNull();
    expect(lockSquare(rect!)).toMatchObject({ posX: 0, posY: 0, width: 10, height: 10 });
  });
});

describe('boxesInside', () => {
  it('keeps a box fully inside a 10×10 band and drops one that crosses the edge', () => {
    const band = { posX: 0, posY: 0, width: 10, height: 10 };
    expect(
      boxesInside(band, [
        { id: 'in', posX: 2, posY: 2, width: 4, height: 4 },
        { id: 'out', posX: 8, posY: 8, width: 4, height: 4 },
      ]),
    ).toEqual(['in']);
  });

  it('keeps two of three entries when the third crosses the edge', () => {
    const band = { posX: 0, posY: 0, width: 10, height: 10 };
    expect(
      boxesInside(band, [
        { id: 'a', posX: 1, posY: 1, width: 3, height: 3 },
        { id: 'b', posX: 5, posY: 5, width: 3, height: 3 },
        { id: 'edge', posX: 8, posY: 8, width: 4, height: 4 },
      ]),
    ).toEqual(['a', 'b']);
  });
});


describe('PX_PER_MM', () => {
  it('matches the canvas millimetre', () => {
    expect(PX_PER_MM).toBe(96 / 25.4);
  });
});
