import type { GestureInput, SheetTool, ToolEffect, ToolHost } from './sheet-tool';
import { entryAtPoint, sampleTextForeground } from './sample-color';

/**
 * Eyedropper. A click samples the topmost box under the pointer into foreground
 * memory. The page entry is not rewritten.
 */
export const colorpickerTool: SheetTool & { id: 'colorpicker' } = {
  id: 'colorpicker',
  label: 'Colorpicker',
  icon: 'fa-solid fa-eye-dropper',
  position: 80,
  key: 'e',
  creates: false,
  cursor: 'crosshair',
  gesture: 'click',
  onGesture(input: GestureInput, host: ToolHost): ToolEffect {
    const page = host.page();
    if (!page) return { kind: 'none' };
    const hit = entryAtPoint(page.contents, input.click);
    if (!hit) return { kind: 'none' };
    const fromText = sampleTextForeground(hit);
    if (fromText) return { kind: 'set-foreground', hex: fromText };
    const fromPicture = host.samplePicture?.(hit.id, input.click) ?? null;
    if (!fromPicture) return { kind: 'none' };
    return { kind: 'set-foreground', hex: fromPicture };
  },
};
