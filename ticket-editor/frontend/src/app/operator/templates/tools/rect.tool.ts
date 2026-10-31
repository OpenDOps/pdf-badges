import { appendRect } from '../page-view';
import { dragTool } from './drag-tool';

export const rectTool = dragTool({
  id: 'rect',
  label: 'Rectangle',
  icon: 'fa-regular fa-square',
  position: 50,
  key: 'r',
  place: (page, rect, host) => appendRect(page, rect, host.style()),
});
