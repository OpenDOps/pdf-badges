export interface PickerProps {
  color: string;
  onChange: (hex: string) => void;
  mode: 'linear' | 'radial';
  size?: number;
}

export function mount(parent: Element, props: PickerProps): void;
