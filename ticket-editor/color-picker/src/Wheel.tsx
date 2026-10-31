import { h } from 'preact';
import { controlSize, hexFromWheel, hexToHsv, WHEEL_GRADIENT, wheelPoint } from './color';
import { DragSurface, type Point } from './DragSurface';
import type { ColorProps } from './Box';

/** Hue around the wheel, saturation out from the center. Value stays. */
export class Wheel extends DragSurface<ColorProps> {
  protected moveTo(point: Point): void {
    const next = hexFromWheel(this.props.color, point.x, point.y, this.props.size);
    this.place(wheelPoint(next, this.props.size));
    this.props.onChange(next);
  }

  render() {
    const size = controlSize(this.props.size);
    const hsv = hexToHsv(this.props.color);
    const shade = 1 - hsv.v;
    return this.surface(
      size,
      size,
      `linear-gradient(rgba(0,0,0,${shade}), rgba(0,0,0,${shade})), radial-gradient(circle closest-side, #fff, transparent), ${WHEEL_GRADIENT}`,
      wheelPoint(this.props.color, size),
      'Hue and saturation',
      true,
    );
  }
}
