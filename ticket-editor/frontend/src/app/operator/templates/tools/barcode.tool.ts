import { appendCode } from '../page-view';
import { dragTool } from './drag-tool';

export const barcodeTool = dragTool({
  id: 'barcode',
  label: 'Barcode',
  icon: 'fa-solid fa-barcode',
  position: 70,
  key: 'b',
  place: (page, rect) => appendCode(page, rect, 'ean13'),
});
