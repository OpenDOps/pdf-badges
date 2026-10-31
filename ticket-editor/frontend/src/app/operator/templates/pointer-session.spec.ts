import { PX_PER_MM } from './draw-tool';
import { DrawPointerSession, type DrawMode } from './pointer-session';

function pointer(type: string, xMm: number, yMm: number): PointerEvent {
  return new PointerEvent(type, {
    clientX: xMm * PX_PER_MM,
    clientY: yMm * PX_PER_MM,
    bubbles: true,
    cancelable: true,
    button: 0,
  });
}

const canvas: DrawMode = { gesture: 'canvas', cursor: null };
const crosshair: DrawMode = { gesture: 'drag', cursor: 'crosshair' };
const typing: DrawMode = { gesture: 'click', cursor: 'text' };

describe('pointer session', () => {
  function session() {
    const doc = document.implementation.createHTMLDocument('sheet');
    const drawn: { rect: unknown; click: unknown; shift: boolean }[] = [];
    const draw = new DrawPointerSession(doc, () => ({ x: 0, y: 0 }), (rect, click, shift) => drawn.push({ rect, click, shift }));
    return { doc, drawn, draw };
  }

  it('installs listeners for click and drag, and removes them for canvas', () => {
    const { draw } = session();
    expect(draw.listening).toBe(false);
    draw.setTool(canvas);
    expect(draw.listening).toBe(false);
    draw.setTool(typing);
    expect(draw.listening).toBe(true);
    draw.setTool(crosshair);
    expect(draw.listening).toBe(true);
    draw.setTool(canvas);
    expect(draw.listening).toBe(false);
  });

  it('sets the crosshair for a drag and clears it for canvas', () => {
    const { doc, draw } = session();
    draw.setTool(crosshair);
    expect(doc.documentElement.getAttribute('data-draw-cursor')).toBe('crosshair');
    expect(doc.querySelector('style[data-draw-cursor]')?.textContent).toContain('cursor: crosshair');
    draw.setTool({ gesture: 'drag', cursor: null });
    expect(doc.documentElement.hasAttribute('data-draw-cursor')).toBe(false);
    draw.setTool(canvas);
    expect(doc.querySelector('style[data-draw-cursor]')).toBeNull();
  });

  it('sets the typing pointer for a click', () => {
    const { doc, draw } = session();
    draw.setTool(typing);
    expect(doc.documentElement.getAttribute('data-draw-cursor')).toBe('text');
    expect(doc.querySelector('style[data-draw-cursor]')?.textContent).toContain('cursor: text');
    draw.setTool(crosshair);
    expect(doc.documentElement.getAttribute('data-draw-cursor')).toBe('crosshair');
    draw.setTool(canvas);
    expect(doc.querySelector('style[data-draw-cursor]')).toBeNull();
  });

  it('reads the rectangle from pointer down, move, and up', () => {
    const { doc, drawn, draw } = session();
    const down = pointer('pointerdown', 10, 8);
    const move = pointer('pointermove', 7, 14);
    const up = pointer('pointerup', 4, 20);

    draw.setTool(canvas);
    doc.dispatchEvent(down);
    doc.dispatchEvent(move);
    doc.dispatchEvent(up);
    expect(drawn).toEqual([]);
    expect(down.defaultPrevented).toBe(false);
    expect(doc.querySelector('[data-draw-band]')).toBeNull();

    draw.setTool(crosshair);
    doc.dispatchEvent(pointer('pointerdown', 10, 8));
    const dragging = pointer('pointermove', 7, 14);
    doc.dispatchEvent(dragging);
    const band = doc.querySelector('[data-draw-band]') as HTMLElement;
    expect(band).not.toBeNull();
    expect(band.style.border).toContain('dashed');
    expect(band.style.left).toBe('7mm');
    expect(band.style.top).toBe('8mm');
    expect(band.style.width).toBe('3mm');
    expect(band.style.height).toBe('6mm');
    expect(dragging.defaultPrevented).toBe(true);

    const release = pointer('pointerup', 4, 20);
    doc.dispatchEvent(release);
    expect(drawn).toEqual([
      {
        rect: { posX: 4, posY: 8, width: 6, height: 12, anchorX: 10, anchorY: 8 },
        click: { x: 10, y: 8 },
        shift: false,
      },
    ]);
    expect(release.defaultPrevented).toBe(true);
    expect(doc.querySelector('[data-draw-band]')).toBeNull();
  });

  it('returns null when a side is under 2 mm and a snapped rect when both sides clear it', () => {
    const { doc, drawn, draw } = session();
    draw.setTool(crosshair);
    doc.dispatchEvent(pointer('pointerdown', 1, 1));
    doc.dispatchEvent(pointer('pointermove', 1.2, 8));
    doc.dispatchEvent(pointer('pointerup', 1.2, 8));
    expect(drawn).toEqual([{ rect: null, click: { x: 1, y: 1 }, shift: false }]);
    expect(doc.querySelector('[data-draw-band]')).toBeNull();

    doc.dispatchEvent(pointer('pointerdown', 1, 1));
    doc.dispatchEvent(pointer('pointerup', 12, 9));
    expect(drawn[1]).toEqual({
      rect: { posX: 1, posY: 1, width: 11, height: 8, anchorX: 1, anchorY: 1 },
      click: { x: 1, y: 1 },
      shift: false,
    });
  });

  it('finishes a click with no rectangle and the snapped pointer-down point', () => {
    const { doc, drawn, draw } = session();
    draw.setTool(typing);
    doc.dispatchEvent(pointer('pointerdown', 3, 4));
    doc.dispatchEvent(pointer('pointermove', 3.4, 4.2));
    expect(doc.querySelector('[data-draw-band]')).toBeNull();
    doc.dispatchEvent(pointer('pointerup', 3.4, 4.2));
    expect(drawn).toEqual([{ rect: null, click: { x: 3, y: 4 }, shift: false }]);
  });

  it('does not finish when pointer down yields', () => {
    const { doc, drawn, draw } = session();
    const down = pointer('pointerdown', 4, 5);
    draw.setTool({ ...crosshair, onPointerDown: () => 'yield' });
    doc.dispatchEvent(down);
    doc.dispatchEvent(pointer('pointermove', 20, 20));
    doc.dispatchEvent(pointer('pointerup', 20, 20));
    expect(drawn).toEqual([]);
    expect(down.defaultPrevented).toBe(false);
    expect(doc.querySelector('[data-draw-band]')).toBeNull();
  });

  it('reports Shift on pointer up', () => {
    const { doc, drawn, draw } = session();
    draw.setTool(crosshair);
    const down = pointer('pointerdown', 0, 0);
    doc.dispatchEvent(down);
    doc.dispatchEvent(pointer('pointermove', 30, 12));
    const up = new PointerEvent('pointerup', {
      clientX: 30 * PX_PER_MM,
      clientY: 12 * PX_PER_MM,
      bubbles: true,
      cancelable: true,
      button: 0,
      shiftKey: true,
    });
    doc.dispatchEvent(up);
    expect(drawn).toEqual([
      {
        rect: { posX: 0, posY: 0, width: 30, height: 12, anchorX: 0, anchorY: 0 },
        click: { x: 0, y: 0 },
        shift: true,
      },
    ]);
  });
});
