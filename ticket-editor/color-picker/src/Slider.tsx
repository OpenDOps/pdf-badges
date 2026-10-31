import { h } from 'preact';
import { controlSize, hexFromSlider, hexToHsv, hsvToHex, sliderPoint } from './color';
import { DragSurface, type Point } from './DragSurface';
import type { ColorProps } from './Box';

export interface SliderProps extends ColorProps {
  channel: 'hue' | 'value';
}

/** Horizontal hue or value strip. The other channels stay. */
export class Slider extends DragSurface<SliderProps> {
  protected moveTo(point: Point): void {
    const next = hexFromSlider(this.props.color, this.props.channel, point.x, this.props.size);
    this.place({ x: sliderPoint(next, this.props.channel, this.props.size).x, y: 8 });
    this.props.onChange(next);
  }

  render() {
    const size = controlSize(this.props.size);
    const hsv = hexToHsv(this.props.color);
    const handle = sliderPoint(this.props.color, this.props.channel, size);
    const background = this.props.channel === 'hue'
      ? 'linear-gradient(to right, red, yellow, lime, aqua, blue, magenta, red)'
      : `linear-gradient(to right, #000, ${hsvToHex({ h: hsv.h, s: hsv.s, v: 1 })})`;
    return this.surface(size, 16, background, { x: handle.x, y: 8 }, this.props.channel === 'hue' ? 'Hue' : 'Value');
  }
}
