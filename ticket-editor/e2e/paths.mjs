import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));

export const ticketEditorRoot = path.join(here, '..');
export const templatesDir = path.join(ticketEditorRoot, '.e2e-templates');
export const frontendRoot = path.join(ticketEditorRoot, 'frontend');
