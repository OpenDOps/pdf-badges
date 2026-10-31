import { dragTool } from './drag-tool';

/** The drag is kept. Nothing is appended until the image source dialog exists. */
export const imageTool = dragTool({
  id: 'image',
  label: 'Image',
  icon: 'fa-solid fa-image',
  position: 40,
  key: 'i',
});
