import { h } from 'preact';
import { boxPoint, controlSize, hexFromBox, hsvToHex, hexToHsv } from './color';
import { DragSurface, type Point } from './DragSurface';

export interface ColorProps {
  color: string;
  onChange: (hex: string) => void;
  size?: number;
}

/** Saturation across, value down. Hue stays the hue of `color`. */
export class Box extends DragSurface<ColorProps> {
  protected moveTo(point: Point): void {
    const next = hexFromBox(this.props.color, point.x, point.y, this.props.size);
    this.place(boxPoint(next, this.props.size));
    this.props.onChange(next);
  }

  render() {
    const size = controlSize(this.props.size);
    const hsv = hexToHsv(this.props.color);
    const hue = hsvToHex({ h: hsv.h, s: 1, v: 1 });
    return this.surface(
      size,
      size,
      `linear-gradient(to top, #000, transparent), linear-gradient(to right, #fff, ${hue})`,
      boxPoint(this.props.color, size),
      'Saturation and value',
    );
  }
}
