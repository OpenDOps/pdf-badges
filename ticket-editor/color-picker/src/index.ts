import { h, render } from 'preact';
import { ColorPicker, type PickerProps } from './ColorPicker';

export { Box } from './Box';
export { Wheel } from './Wheel';
export { Slider } from './Slider';
export { ColorPicker, type PickerProps } from './ColorPicker';
export {
  boxPoint,
  hexFromBox,
  hexFromSlider,
  hexFromWheel,
  sliderPoint,
  wheelPoint,
} from './color';

export function mount(parent: Element, props: PickerProps): void {
  render(h(ColorPicker, props), parent);
}
