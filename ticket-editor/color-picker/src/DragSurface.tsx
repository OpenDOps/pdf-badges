import { Component, h } from 'preact';

export interface Point {
  x: number;
  y: number;
}

/** Mouse and touch drag. The root element is `this.base` on both builds. */
export abstract class DragSurface<P> extends Component<P> {
  private knob: HTMLElement | null = null;

  protected abstract moveTo(point: Point): void;

  /** The circle follows the pointer before the parent renders again. */
  protected place(handle: Point): void {
    if (!this.knob) return;
    this.knob.style.left = `${handle.x}px`;
    this.knob.style.top = `${handle.y}px`;
  }

  handleEvent = (event: Event) => {
    if (event.type === 'mousedown' || event.type === 'touchstart') {
      const point = this.pointOf(event);
      if (!point) return;
      event.preventDefault();
      this.moveTo(point);
      document.addEventListener('mousemove', this);
      document.addEventListener('mouseup', this);
      document.addEventListener('touchmove', this, { passive: false });
      document.addEventListener('touchend', this);
      return;
    }
    if (event.type === 'mousemove' || event.type === 'touchmove') {
      event.preventDefault();
      const point = this.pointOf(event);
      if (point) this.moveTo(point);
      return;
    }
    document.removeEventListener('mousemove', this);
    document.removeEventListener('mouseup', this);
    document.removeEventListener('touchmove', this);
    document.removeEventListener('touchend', this);
  };

  protected surface(width: number, height: number, background: string, handle: Point, label: string, round = false): h.JSX.Element {
    const touch = { ontouchstart: this.handleEvent } as h.JSX.HTMLAttributes<HTMLDivElement>;
    return (
      <div
        className="cp-surface"
        role="slider"
        aria-label={label}
        style={{
          position: 'relative',
          width,
          height,
          borderRadius: round ? '50%' : 0,
          touchAction: 'none',
          userSelect: 'none',
          background,
        }}
        onMouseDown={this.handleEvent}
        {...touch}
      >
        <span
          ref={(el: HTMLElement | null) => {
            this.knob = el;
          }}
          style={{
            position: 'absolute',
            left: handle.x,
            top: handle.y,
            width: 12,
            height: 12,
            marginLeft: -6,
            marginTop: -6,
            borderRadius: '50%',
            border: '2px solid #fff',
            boxShadow: '0 0 0 1px #000',
            pointerEvents: 'none',
          }}
        />
      </div>
    );
  }

  private pointOf(event: Event): Point | null {
    const surface = this.base instanceof HTMLElement ? this.base : null;
    if (!surface) return null;
    const bounds = surface.getBoundingClientRect();
    const touchEvent = event as TouchEvent;
    const touch = touchEvent.touches?.[0] ?? touchEvent.changedTouches?.[0];
    const mouse = event as MouseEvent;
    const clientX = touch ? touch.clientX : mouse.clientX;
    const clientY = touch ? touch.clientY : mouse.clientY;
    if (!Number.isFinite(clientX) || !Number.isFinite(clientY)) return null;
    return { x: clientX - bounds.left, y: clientY - bounds.top };
  }
}
