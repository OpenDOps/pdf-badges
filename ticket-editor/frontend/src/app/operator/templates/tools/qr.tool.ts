import { placedCodeRect } from '../draw-tool';
import { appendCode } from '../page-view';
import { dragTool } from './drag-tool';

export const qrTool = dragTool({
  id: 'qr',
  label: 'QR',
  icon: 'fa-solid fa-qrcode',
  position: 60,
  key: 'q',
  adjustRect: (rect, shift) => placedCodeRect(rect, 'qr', shift),
  place: (page, rect) => appendCode(page, rect, 'qr'),
});
