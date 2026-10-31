/** Page-size catalog. The editor reads this object. It does not fetch it. */

export type PageFormatName = 'A4' | 'A5' | 'A6' | 'Letter' | 'Legal' | 'custom';
export type Orientation = 'portrait' | 'landscape';

export interface PageFormatOption {
  format: PageFormatName;
  width_mm: number;
  height_mm: number;
}

export interface ThermalPreset {
  label: string;
  width_mm: number;
  height_mm: number;
}

export interface PageFormats {
  standard: PageFormatOption[];
  thermal_presets: ThermalPreset[];
  custom: { min_mm: number; max_mm: number };
  orientations: Orientation[];
  default_new_template: { format: PageFormatName; width_mm: number; height_mm: number };
}

export const pageFormats: PageFormats = {
  standard: [
    { format: 'A4', width_mm: 210, height_mm: 297 },
    { format: 'A5', width_mm: 148, height_mm: 210 },
    { format: 'A6', width_mm: 105, height_mm: 148 },
    { format: 'Letter', width_mm: 215.9, height_mm: 279.4 },
    { format: 'Legal', width_mm: 215.9, height_mm: 355.6 },
  ],
  thermal_presets: [
    { label: '100×100 мм (бейдж Zebra)', width_mm: 100, height_mm: 100 },
    { label: '100×150 мм', width_mm: 100, height_mm: 150 },
    { label: '100×70 мм', width_mm: 100, height_mm: 70 },
    { label: '58×40 мм', width_mm: 58, height_mm: 40 },
  ],
  custom: { min_mm: 10, max_mm: 1000 },
  orientations: ['portrait', 'landscape'],
  default_new_template: { format: 'custom', width_mm: 100, height_mm: 100 },
};
