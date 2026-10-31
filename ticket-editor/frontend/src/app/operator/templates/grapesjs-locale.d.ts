// grapesjs ships its locale files without type declarations; the editor
// only needs the default export as an opaque messages object.
declare module 'grapesjs/locale/ru.mjs' {
  const messages: Record<string, unknown>;
  export default messages;
}
