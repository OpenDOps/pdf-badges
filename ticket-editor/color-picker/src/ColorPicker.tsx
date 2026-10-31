import { Component, h } from 'preact';
import { Box, type ColorProps } from './Box';
import { Slider } from './Slider';
import { Wheel } from './Wheel';

export interface PickerProps extends ColorProps {
  mode: 'linear' | 'radial';
}

/** Linear is the square plus a hue slider. Radial is the wheel plus a value slider.
 * The handle follows the pointer from local state. A parent that does not pass the
 * new color back still sees the circle move. */
export class ColorPicker extends Component<PickerProps> {
  private live = '';
  private seen = '';

  private emit = (hex: string) => {
    this.live = hex;
    this.forceUpdate();
    this.props.onChange(hex);
  };

  render() {
    if (this.props.color !== this.seen) {
      this.seen = this.props.color;
      this.live = this.props.color;
    }
    const color = this.live || this.props.color;
    const onChange = this.emit;
    if (this.props.mode === 'radial') {
      return (
        <div className="cp-picker">
          <Wheel color={color} onChange={onChange} size={this.props.size} />
          <Slider color={color} onChange={onChange} size={this.props.size} channel="value" />
        </div>
      );
    }
    return (
      <div className="cp-picker">
        <Box color={color} onChange={onChange} size={this.props.size} />
        <Slider color={color} onChange={onChange} size={this.props.size} channel="hue" />
      </div>
    );
  }
}
