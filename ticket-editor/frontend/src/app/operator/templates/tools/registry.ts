import { barcodeTool } from './barcode.tool';
import { colorpickerTool } from './colorpicker.tool';
import { imageTool } from './image.tool';
import { marqueeTool } from './marquee.tool';
import { qrTool } from './qr.tool';
import { rectTool } from './rect.tool';
import type { SheetTool, ToolEffect } from './sheet-tool';
import { textTool } from './text.tool';

const none = (): ToolEffect => ({ kind: 'none' });

export type EditorTool = 'select' | 'marquee' | 'text' | 'image' | 'rect' | 'qr' | 'barcode' | 'colorpicker';

/** The rail order. A later tool can take a position between these tens. */
export const sheetTools: (SheetTool & { id: EditorTool })[] = [
  { id: 'select', label: 'Select', icon: 'fa-solid fa-arrow-pointer', position: 10, key: 'v', creates: false, cursor: null, gesture: 'canvas', onGesture: none },
  marqueeTool,
  textTool,
  imageTool,
  rectTool,
  qrTool,
  barcodeTool,
  colorpickerTool,
];

/** Ids in rail order. */
export const editorToolIds: EditorTool[] = [...sheetTools]
  .sort((left, right) => left.position - right.position)
  .map((tool) => tool.id);

/** Bare key to tool id. Escape is not here; the rail handles it as Select. */
export const toolShortcuts: Record<string, EditorTool> = Object.fromEntries(
  sheetTools.flatMap((tool) => (tool.key ? [[tool.key, tool.id] as const] : [])),
);

export function toolById(id: EditorTool): SheetTool {
  const found = sheetTools.find((tool) => tool.id === id);
  if (!found) throw new Error(`Unknown tool ${id}`);
  return found;
}

/** Rail order, smallest position first. */
export function toolsInOrder(): (SheetTool & { id: EditorTool })[] {
  return [...sheetTools].sort((left, right) => left.position - right.position);
}
