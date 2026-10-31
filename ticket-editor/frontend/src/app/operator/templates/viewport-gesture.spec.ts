import { viewportAfterPan, viewportAfterPinch } from './viewport-gesture';

const start = {
  zoom: 100,
  coords: { x: 40, y: -20 },
  origin: { x: 10, y: 30 },
};

describe('viewportAfterPinch', () => {
  it('keeps the pose when the fingers have not moved', () => {
    expect(viewportAfterPinch(start, 1, start.origin, 25, 400)).toEqual({
      zoom: 100,
      coords: { x: 40, y: -20 },
    });
  });

  it('zooms toward the fingers and follows them together', () => {
    const next = viewportAfterPinch(start, 2, { x: 16, y: 34 }, 25, 400);
    expect(next.zoom).toBe(200);
    expect(next.coords.x).toBeCloseTo(10 - (10 - 40) * 2 + 6);
    expect(next.coords.y).toBeCloseTo(30 - (30 - -20) * 2 + 4);
  });

  it('keeps a drag that happened before the next pinch step', () => {
    const dragged = {
      ...start,
      coords: viewportAfterPan(start.coords, -30, 12),
    };
    const next = viewportAfterPinch(dragged, 1, { x: 18, y: 26 }, 25, 400);
    expect(next.zoom).toBe(100);
    expect(next.coords).toEqual({ x: dragged.coords.x + 8, y: dragged.coords.y - 4 });
  });

  it('stops at the zoom limits without dropping the pan', () => {
    const next = viewportAfterPinch(start, 10, { x: 12, y: 30 }, 25, 400);
    expect(next.zoom).toBe(400);
    expect(next.coords.x).toBeCloseTo(10 - (10 - 40) * 4 + 2);
    expect(next.coords.y).toBeCloseTo(30 - (30 - -20) * 4);
  });
});

describe('viewportAfterPan', () => {
  it('moves the sheet by the wheel delta', () => {
    expect(viewportAfterPan({ x: 40, y: -20 }, 5, -3)).toEqual({ x: 35, y: -17 });
  });
});
