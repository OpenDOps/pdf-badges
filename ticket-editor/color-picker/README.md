# Color picker

A saturation square, a hue or value slider, and a color wheel. The value in and out is `#RRGGBB`. The components are written once, against `preact`. The build selects Preact or React.

Run the commands from `ticket-editor/color-picker`.

## Build

React, for an app that already loads React. React and React DOM stay as imports. They are not copied into the file.

```bash
node scripts/build.mjs react
```

Preact included in the file. The result imports neither Preact nor React.

```bash
node scripts/build.mjs preact
```

Preact left as an import, for an app that already loads Preact.

```bash
node scripts/build.mjs preact --external
```

All three:

```bash
npm run build
npm test
```

## Import

| Build | Command | Import |
|---|---|---|
| Preact included | `node scripts/build.mjs preact` | `ticket-editor-color-picker` |
| Preact shared | `node scripts/build.mjs preact --external` | `ticket-editor-color-picker/preact` |
| React shared | `node scripts/build.mjs react` | `ticket-editor-color-picker/react` |

```js
import { Box, Wheel, Slider, ColorPicker, mount } from 'ticket-editor-color-picker';
```

`ColorPicker` takes `mode: 'linear'` (square and hue slider) or `mode: 'radial'` (wheel and value slider). `mount(parent, props)` draws it into an element. `Box`, `Wheel`, and `Slider` are the separate controls. Each calls `onChange` with `#RRGGBB`.

## Ticket editor

The Angular editor imports `ticket-editor-color-picker`, the build that includes Preact. Build that file before starting the editor:

```bash
node scripts/build.mjs preact
```

A React editor imports `ticket-editor-color-picker/react` instead. That build does not include Preact. The app's React is the only copy, so Preact is not loaded.
