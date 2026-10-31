import path from 'node:path';
import { createApp, defaultTemplatesDir, pdfProtoPath, pdfToolingAddr } from './app.mjs';

const templatesDir = process.env.TEMPLATES_DIR ? path.resolve(process.env.TEMPLATES_DIR) : defaultTemplatesDir;
const host = process.env.BIND_HOST || '127.0.0.1';
const port = Number(process.env.PORT || 4310);
const pdfAddr = pdfToolingAddr();
const pdfProto = pdfProtoPath();

const app = createApp(templatesDir);
app.locals.pdfToolingAddr = pdfAddr;
app.locals.pdfProto = pdfProto;
app.listen(port, host, () => {
  console.log(`ticket-editor listening on http://${host}:${port}`);
});
