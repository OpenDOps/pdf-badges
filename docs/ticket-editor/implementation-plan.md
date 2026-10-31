# Run the ticket editor without the registration node

The editor is the Angular screen at `PIR_REG/expocab-offline-registration/frontend/src/app/operator/templates/`. Today it opens only after operator login and only talks to the Django node (`http://localhost:8000/api`).

This plan leaves the GrapesJS canvas and the CodeMirror mode in place, drops the login, and puts a small Node process in front of the calls that store a template and its files. Page formats and the demo guest are TypeScript objects in `ticket-editor/src/`. The editor imports them. It does not fetch them. PDF preview, Zebra preview, and test print stay on the Django node. They need the Django template engine and headless Chromium.

`npm start` in the Angular app is already a Vite dev server (`@angular/build:dev-server`). This plan does not rewrite the editor as a second Vite application.

The Node process lives at `ticket-editor/` in this repository. The Angular changes stay in the expocab frontend.

## Steps

Each step has a Check. The check uses only what that step added, so it can pass before [Run](#step-6-run). Run is the full screen, after the pieces are already checked on their own.

[yaml-and-preview-plan.md](yaml-and-preview-plan.md) is the gate in front of [Run](#step-6-run). Steps 1–5 of this file come first. Then that plan, through its own [Run](yaml-and-preview-plan.md#step-18-run). Step 6 of this file starts only after that gate has passed.

| Step | What it does | Status |
|---|---|---|
| [1. Node package, formats, and demo context](#step-1-node-package-formats-and-demo-context) | The process, `pageFormats`, and `demoContext`. | ✅ Completed |
| [2. Templates and assets](#step-2-templates-and-assets) | Read and write `template.json`, and upload, read, and delete files. `npm test` covers the curl sequence. | ✅ Completed |
| [3. Editor route with no login](#step-3-editor-route-with-no-login) | `/editor/:id` opens with no operator token. | ✅ Completed |
| [4. Point the dev build at the Node process](#step-4-point-the-dev-build-at-the-node-process) | `npm run start:editor` loads the sheet from the Node process. | ✅ Completed |
| [5. Plug toolbar actions into the route](#step-5-plug-toolbar-actions-into-the-route) | `/editor` registers no preview or print actions. The operator route keeps the Django cluster. | ✅ Completed |
| [YAML and preview](yaml-and-preview-plan.md) | Gate. Store the page as YAML, preview through pdf-tooling, and pass that plan's [Run](yaml-and-preview-plan.md#step-18-run) before step 6. | Not started |
| [6. Run](#step-6-run) | Both processes, then the full click-through on `/editor/1`. Starts after the gate. | Not started |
| [7. Field pipes and ifSt](#step-7-field-pipes-and-ifst) | Last. `\|upper`, `\|title`, `\|join:", "`, `\|default:"…"`, and the draw rule. Starts after [Run](#step-6-run). | Not started |

Shrink-to-fit is not a step. `fit_text.js` stays on the Django node. This editor defers it. Text fitting will be rewritten with the PDF page (`auto_scale`), not copied across as the canvas script.

## Checking a step

A step is testable when its Check does not need a later step.

| What the step added | How to check it, before Run |
|---|---|
| TypeScript objects | `npm run check` in `ticket-editor/`. No server and no browser. |
| HTTP on the Node process | `npm start` in `ticket-editor/`, then curl. No Angular app. |
| An Angular route or toolbar | `npm start` in the frontend, open that one URL, and look at that one thing. The Node process is required only when the screen loads or saves a template. |

Run repeats those checks together in the browser. It does not replace them. A failure in Run is investigated in the step whose Check already covers that call.

Links in this file go to the heading of the step. The heading text is the address (`#step-1-node-package-formats-and-demo-context` for the first step).

## What the Node process serves

Same paths and JSON shapes the Angular `PrintApiService` already uses (`frontend/src/app/core/models/print.models.ts`). No `Authorization` header. CORS allows the Angular dev origin (`http://localhost:4200`).

| Call | Path | Body |
|---|---|---|
| Load | `GET /api/print_template/:id` | `PrintTemplateDetail` |
| Save | `PUT /api/print_template/:id` | `PrintTemplateWrite`. Reply is the stored detail |
| Upload | `POST /api/print_template/:id/assets` | multipart `name` + `file`. Reply `{ name, path, manifest }` |
| Read file | `GET /api/print_template/:id/assets/:name` | raw bytes, `Content-Type` from the extension |
| Delete file | `DELETE /api/print_template/:id/assets/:name` | `{ manifest }` |

Disk layout:

```text
ticket-editor/
  package.json
  server.mjs
  src/formats.ts
  src/demo-context.ts
  data/templates/<id>/template.json
  data/templates/<id>/files/<name>.<ext>
```

`template.json` is the detail object. `manifest` inside it maps the palette name (`logo`) to the relative path (`files/logo.png`).

## Step 1. Node package, formats, and demo context

[Steps](#steps)

✅ Completed. This is the old steps for the process, `formats()`, and `demoContext()`, done together.

`ticket-editor/package.json` has `"type": "module"` and one dependency, `express`. No database. `server.mjs` listens on `127.0.0.1:4310`. It creates `data/templates/` on startup. Every response sets `Access-Control-Allow-Origin` to the Angular origin and allows `GET`, `PUT`, `POST`, `DELETE`, and `Content-Type`. A missing template id returns 404 with `{ "detail": "not found" }`. Run script: `"start": "node server.mjs"`.

`pageFormats` in `ticket-editor/src/formats.ts` is the catalog copied from `backend/registration/printing/page_formats.py` (`formats_catalog()`). The editor imports that object. There is no `GET /api/print_template/formats`.

Named sizes, portrait millimetres: A4 210×297, A5 148×210, A6 105×148, Letter 215.9×279.4, Legal 215.9×355.6. Thermal presets for a custom sheet: 100×100, 100×150, 100×70, 58×40. Custom sides run from 10 to 1000 mm. Orientations are `portrait` and `landscape`. A new template defaults to custom 100×100. The editor uses this only to fill the page-size dropdown. Saving a page writes `page_format`, `page_width_mm`, and `page_height_mm` onto the template row in [Templates and assets](#step-2-templates-and-assets). `roll` is a stored choice in Django and is absent from this catalog. The editor rejects it.

`demoContext` in `ticket-editor/src/demo-context.ts` is a fixed guest. The editor imports that object. There is no `GET /api/print_template/demo_context`, and the editor does not request a Formly schema. The palette is built from `fields`. The canvas substitutes `form_data` and `ticket` into variable blocks, and uses `codes` as the image `src` for barcode and QR blocks. `geometry` sizes those blocks in printer dots.

Field keys stay `field_<n>` so a template authored here still matches the Django context (`form_data.field_1`) if it is later copied onto a node.

| Key | Label | Sample |
|---|---|---|
| `field_1` | Фамилия | Петрова |
| `field_2` | Имя | Анна |
| `field_3` | Компания | ООО «Ромашка-Экспо» |
| `field_4` | Должность | Руководитель отдела маркетинга |
| `field_5` | Теги | Доклад, Стенд (a list) |
| `field_6` | Спикер | boolean `true` |

`ticket` is number `A-00042`, code `2000000000008`, type name `Standard`. `has_vcard` is false, so the vCard palette item stays disabled and the two vCard code strings are empty.

`codes.barcode`, `codes.barcode_svg`, `codes.qrcode`, and `codes.qrcode_svg` are data URIs for that EAN-13 and a QR of the same code. `geometry.ean13_modules` is 117. `geometry.qr_modules` is 29. The canvas snaps a QR or EAN-13 width to a whole number of printer dots per module using this count and 203 dpi.

`field_5` is a list so `|join` has a value to join. `field_6` is a boolean so `ifSt` can be checked against `true`. Those pipes and `ifSt` are [Field pipes and ifSt](#step-7-field-pipes-and-ifst), after the editor is running.

### Check

No browser. From `ticket-editor/`:

```bash
npm run check
npm start
```

In another terminal:

```bash
curl -sS -D - http://127.0.0.1:4310/api/print_template/nope
```

`npm run check` exits 0. That is `pageFormats` and `demoContext` typechecking. `Access-Control-Allow-Origin` is `http://localhost:4200`. A non-numeric id, `GET /api/print_template/nope`, is 404 `{"detail":"not found"}`. `data/templates/` exists. Stop the process when the curl returns. After [step 2](#step-2-templates-and-assets), `GET /api/print_template/1` is the blank template, so this check uses `nope` rather than `1`.

## Step 2. Templates and assets

[Steps](#steps)

✅ Completed. One step for the template row and its files, against the process from [step 1](#step-1-node-package-formats-and-demo-context). The Angular app is not running. `npm test` runs the same checks without binding port 4310.

On startup, if `data/templates/1/template.json` is missing, write a blank local template:

```json
{
  "id": 1,
  "ticket_type": 1,
  "ticket_type_name": "Standard",
  "name": "Badge",
  "source": "local",
  "is_default": true,
  "html": "<!doctype html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<style>\nbody { margin: 0; position: relative; width: 100mm; height: 100mm; overflow: hidden; background: #fff; }\n</style>\n</head>\n<body>\n</body>\n</html>\n",
  "editor_state": null,
  "manifest": {},
  "created_at": "",
  "updated_at": "",
  "updated_by_name": "",
  "page_format": "custom",
  "page_width_mm": 100,
  "page_height_mm": 100,
  "orientation": "portrait",
  "margin_top_mm": 0,
  "margin_right_mm": 0,
  "margin_bottom_mm": 0,
  "margin_left_mm": 0
}
```

`GET /api/print_template/:id` reads that file.

`PUT /api/print_template/:id` merges the write body onto the stored object. Accepted keys, matching `PrintTemplateWrite`:

`name`, `html`, `editor_state`, `page_format`, `page_width_mm`, `page_height_mm`, `orientation`, `margin_top_mm`, `margin_right_mm`, `margin_bottom_mm`, `margin_left_mm`, `updated_at`.

Set `updated_at` to an ISO timestamp and return the full detail. Ignore the optimistic-lock check: one writer, no 409. Keep `source: "local"` so the editor stays editable (`source: "cloud"` is read-only in the UI).

Validate page size the way `page_spec()` does: each side from 10 to 1000 mm, margins `>= 0`, and `left + right` less than the width, `top + bottom` less than the height. A failed check returns 400 `{ "detail": "..." }` and does not write the file.

`uploadAsset`, `getAsset`, and `deleteAsset` are the template's own files: a logo, a background, a web font. The HTML refers to them as `{{ files.<name> }}`. The name is the manifest key, not the filename.

`POST /api/print_template/:id/assets` reads multipart fields `file` and `name`. Rules copied from `template_files.py`:

- `name` matches `^[\w.-]{1,100}$`. Empty `name` falls back to the upload filename without its extension.
- Extension is one of `.png`, `.jpg`, `.jpeg`, `.gif`, `.svg`, `.webp`, `.ttf`, `.otf`, `.woff`, `.woff2`, `.css`.
- Size at most 10 MB.
- Write `data/templates/<id>/files/<name><ext>`.
- Set `manifest[name] = "files/<name><ext>"` and save `template.json`.
- Replacing the same `name` deletes the previous file first.
- Reply `201 { "name", "path", "manifest" }`.

`GET /api/print_template/:id/assets/:name` looks up `manifest[name]`, reads that file, and returns the bytes with `Cache-Control: no-store`. 404 when the key is missing.

`DELETE` removes the file and the manifest key, then returns `{ "manifest" }`.

The editor turns each GET into an object URL for the canvas (`fileUrls`). The stored HTML keeps `src="{{ files.logo }}"`. The Node process does not rewrite those URLs.

### Check

From `ticket-editor/`:

```bash
npm test
```

That runs `app.test.mjs`. It seeds a blank template in a temporary directory, then checks the calls below: the first read is 100×100, the 90×130 write is stored, a 5 mm side is 400 and leaves 90×130, the upload mentions `logo`, the asset read returns those bytes with `Cache-Control: no-store`, and delete leaves `manifest` `{}` with a following 404.

The same sequence by hand, with `npm start` in `ticket-editor/`:

```bash
curl -sS http://127.0.0.1:4310/api/print_template/1
curl -sS -X PUT http://127.0.0.1:4310/api/print_template/1 \
  -H 'Content-Type: application/json' \
  -d '{"name":"Badge","page_width_mm":90,"page_height_mm":130}'
curl -sS -X PUT http://127.0.0.1:4310/api/print_template/1 \
  -H 'Content-Type: application/json' \
  -d '{"page_width_mm":5}'
printf 'PNG' > /tmp/logo.png
curl -sS -F name=logo -F file=@/tmp/logo.png http://127.0.0.1:4310/api/print_template/1/assets
curl -sS -D - -o /tmp/logo.out http://127.0.0.1:4310/api/print_template/1/assets/logo
curl -sS -X DELETE http://127.0.0.1:4310/api/print_template/1/assets/logo
```

The first GET is the blank 100×100 template, not 404. The first PUT returns `page_width_mm` 90 and `page_height_mm` 130, and `data/templates/1/template.json` matches. The second PUT is 400 and the file still has 90×130. The upload reply contains `"logo"`. The asset GET returns the uploaded bytes and `Cache-Control: no-store`. After DELETE, `manifest` is `{}` and a second GET of the asset is 404.

A real PNG is not required for this check. The extension rule is what matters, so use a file whose name ends in `.png`.

## Step 3. Editor route with no login

[Steps](#steps)

✅ Completed. In the Angular app, add a route that does not use `setupGuard` or `operatorGuard`:

```text
/editor/:id    EditorLayoutComponent → TemplateEditorComponent
```

Default the empty path `/editor` to `/editor/1`.

`TemplateEditorComponent` currently sends a missing template to `/operator/templates`. On this route, keep the user on `/editor/1` and show the error in the toolbar.

On `/editor/:id` the screen reads `pageFormats` and `demoContext` from `ticket-editor/src/`. It does not call `formats()`, `demoContext()`, or the Formly schema endpoint. Leave `/operator/templates/:id` as it is. The registration node is unchanged.

`auth.interceptor.ts` adds `Authorization` only when a token is already in the browser. This route never logs in, so the header stays off.

### Check

`npx ng test --watch=false --browsers=ChromeHeadless --include=src/app/operator/templates/template-editor.component.spec.ts` in the frontend. It opens `/editor` with no token, stays on `/editor/1`, and checks A4, 100×100, Фамилия, and Петрова without calling `formats()`, `demoContext()`, or Formly.

The same check in a browser. Frontend only. Do not start the Node process, and do not log in.

```bash
cd PIR_REG/expocab-offline-registration/frontend && npm start
```

Open `http://localhost:4200/editor/1` in a browser with no operator token. The address stays `/editor/1`. It does not go to `/operator/login`. The format list includes A4 and 100×100, and the palette shows Фамилия with sample Петрова. Those come from the TypeScript objects, so a missing template on Django does not empty them. The sheet itself may say the template was not found. Loading the saved HTML is [the next step](#step-4-point-the-dev-build-at-the-node-process).

## Step 4. Point the dev build at the Node process

[Steps](#steps)

✅ Completed. Add `src/environments/environment.editor.ts`:

```ts
export const environment = {
  production: false,
  apiBaseUrl: 'http://127.0.0.1:4310/api',
  kioskAutoReset: false,
  publicLink: false,
};
```

Add an Angular configuration `editor` that replaces `environment.ts` with that file, and a script:

```text
"start:editor": "ng serve --configuration editor"
```

Open `http://localhost:4200/editor/1`.

The format dropdown and the palette come from the TypeScript objects. `getTemplate(1)` must succeed before a saved sheet can load. `workstation.refresh()` calls `GET /api/printer/list`, which this process does not implement. The editor already ignores that error and uses 203 dpi. The printer dropdown stays empty. Test print stays disabled because it needs a `printer_id`.

### Check

Stop any frontend already bound to port 4200 (`npm start` uses the Django API). Two terminals, from the repository root.

Terminal 1, the Node process from [step 2](#step-2-templates-and-assets):

```bash
cd ticket-editor && npm start
```

Terminal 2:

```bash
cd PIR_REG/expocab-offline-registration/frontend && npm run start:editor
```

`npm run start:editor` is `ng serve --configuration editor`. To compile without serving:

```bash
cd PIR_REG/expocab-offline-registration/frontend && npx ng build --configuration editor
```

Confirm what is stored, then open the screen:

```bash
curl -sS http://127.0.0.1:4310/api/print_template/1
```

Open `http://localhost:4200/editor/1`. The address stays `/editor/1`. The toolbar name is the stored `name`. The sheet is that file's size (100×100, or 90×130 if the step 2 PUT is still on disk). In the browser network log, template and printer calls go to `127.0.0.1:4310`. None go to `localhost:8000`. `GET /api/printer/list` is 404; the editor ignores it and uses 203 dpi. The printer dropdown stays empty, and test print stays disabled.

## Step 5. Plug toolbar actions into the route

[Steps](#steps)

✅ Completed.

`TemplateEditorComponent` stays the base screen for both routes. It keeps Save, the page-size controls, both edit modes, the palette, and asset upload. It does not call preview, test print, export, or `GET /api/printer/list`.

Those controls are a contribution. A contribution is a toolbar component and an optional aside component:

```ts
export const EDITOR_CONTRIBUTIONS = new InjectionToken<EditorContribution[]>('EDITOR_CONTRIBUTIONS');

export interface EditorContribution {
  toolbar: Type<unknown>;
  aside?: Type<unknown>;
}
```

The editor reads the token with `inject(EDITOR_CONTRIBUTIONS, { optional: true })` and renders each `toolbar` in the toolbar, to the left of Save, with `NgComponentOutlet`. Each `aside` goes in the workspace row, where the zebra bitmap sits now. The aside host is `display: contents`, and the component draws its panel only while it has something to show.

A contribution reads the open sheet through `EDITOR_SHEET`, provided by the editor with `useExisting: TemplateEditorComponent`. The sheet exposes `currentHtml()`, the page size, the template id and name, whether there are unsaved edits, and `showError()`. The method is `currentHtml()` because the editor already stores the document in an `html` field. The contribution does its own HTTP.

The Django cluster is one contribution: the registration-number field, Превью PDF, Как на Zebra, the printer dropdown, Тест-печать, Экспорт, and the mono preview aside. `workstation.refresh()` moves into that component. Register it only on `operator/templates/:id`:

```ts
providers: [
  DjangoPrintState,
  { provide: EDITOR_CONTRIBUTIONS, multi: true, useValue: djangoPrintContribution },
]
```

`/editor/:id` registers nothing. The buttons are absent because the list is empty.

How to put a print or preview toolbar back on a route, including the Django cluster and a new preview button, is [toolbar-contributions.md](toolbar-contributions.md). The buttons stay out of `TemplateEditorComponent`.

The PDF download in [yaml-and-preview-plan.md](yaml-and-preview-plan.md) is a later contribution on `/editor/:id` only. It does not put this Django cluster back. Follow [toolbar-contributions.md](toolbar-contributions.md#example-a-preview-button-of-your-own).

### Check

Same two processes as [step 4](#step-4-point-the-dev-build-at-the-node-process). On `/editor/1` the page has Save and the page-size control. It does not have Превью PDF, Как на Zebra, Тест-печать, Экспорт, the registration-number field, or the printer dropdown. The network log has no `POST /api/print_template/preview`, no test-print call, no export, and no `GET /api/printer/list`.

`/operator/templates/:id` on a normal `npm start` still shows that Django cluster, and those calls still go to `localhost:8000`.

## Step 6. Run

[Steps](#steps)

This step waits on the gate. Implement [yaml-and-preview-plan.md](yaml-and-preview-plan.md) first and pass it, through that plan's [Run](yaml-and-preview-plan.md#step-18-run). Steps 1–5 of this file are already checked. A failure here, after that gate, is a wiring failure, not the first time a call is tried.

The screen is the one that gate left behind. Start it the same way when the stack is not already up:

```bash
cd ticket-editor && docker compose up --build -d
cd PIR_REG/expocab-offline-registration/frontend && npm run start:editor
```

Then open `http://localhost:4200/editor/1`.

Check, in order:

1. The sheet is 100×100 mm and the format dropdown lists A4 and the thermal presets.
2. The palette shows Фамилия, Имя, Компания, Должность, Теги, Спикер, and the ticket fields, with the demo samples.
3. Dropping a variable shows the sample on the canvas (`Петрова`), and Save writes `{{field_1}}` into `data/templates/1/template.yml`. The gate replaced `template.json`.
4. Reload keeps the block.
5. Upload a PNG named `logo`, drop it, see it on the sheet. Save writes an image resource named `logo` under `files/`. Delete removes it from the palette and from the page.

## Step 7. Field pipes and ifSt

[Steps](#steps)

This step is last. It starts only after [Run](#step-6-run).

A variable is still one Django expression. The Node process stores it as written and, when it substitutes a demo value, applies the pipes below. The name used to look up `form_data` or `ticket` is the text before the first `|`.

| Pipe | Expression | Result for the fixture |
|---|---|---|
| `\|upper` | `{{ form_data.field_1\|upper }}` | `ПЕТРОВА` |
| `\|title` | `{{ form_data.field_4\|title }}` | `Руководитель Отдела Маркетинга` |
| `\|join:", "` | `{{ form_data.field_5\|join:", " }}` | `Доклад, Стенд` |
| `\|default:"Гость"` | `{{ form_data.field_1\|default:"Гость" }}` | `Петрова` (the sample is present) |

`|upper` uppercases the whole string. `|title` uppercases the first letter of each word and lowercases the rest. `|join:", "` is for a list: the argument is the separator, quotes included, as the editor writes it. A string value is left unchanged. `|default:"Гость"` substitutes that string when the value is missing, `""`, an empty list, boolean `false`, or integer `0`. A present value is left as it is, so Фамилия stays `Петрова`.

Pipes apply left to right in the order `variableExpression` writes them: join, then default, then case. `{{ form_data.field_5|join:", "|upper }}` is `ДОКЛАД, СТЕНД`. An empty value with `{{ form_data.field_1|default:"Гость"|upper }}` is `ГОСТЬ`.

The canvas already rewrites `|upper`, `|title`, and `|default` for display. This step makes the Node substitution use the same four pipes, including join, so a later preview and a YAML export see one result.

`ifSt` is a condition on the box, stored on the HTML element as `data-if` and copied to the YAML entry as `ifSt`. An absent or empty `ifSt` draws the box. Any other string is a key in the row (`form_data` for this fixture). The condition is true, and the box is drawn, when that value is non-empty, and when the value is boolean `true`. The condition is false, and the box is not drawn, when the value is integer `0` or boolean `false`.

| Row value for the `ifSt` key | Draw |
|---|---|
| missing | no |
| `""` | no |
| boolean `false` | no |
| integer `0` | no |
| boolean `true` | yes |
| any other non-empty value | yes |

A false condition hides the block on the canvas. The saved HTML keeps `data-if`. `field_6: true` draws a block whose `data-if` is `field_6`. Setting that fixture value to `false` or `0` hides it.

### Check

The editor from [Run](#step-6-run) is open.

1. A variable set to `|upper` shows `ПЕТРОВА`. `|title` on the job title capitalises each word. `|join:", "` on Теги shows `Доклад, Стенд`. `|default:"Гость"` on Фамилия still shows `Петрова`. With that sample cleared to empty, the same pipe shows `Гость`, and `|default:"Гость"|upper` shows `ГОСТЬ`.
2. A block with `data-if="field_6"` is visible while the fixture is `true`. It is hidden when that value is boolean `false` or integer `0`.

## Out of this cut

- Operator login, registration list, printers, sync.
- Shrink-to-fit (`fit_text.js`, `data-fit`). Deferred. The PDF page uses `auto_scale` instead, and that replacement is separate work.
- Django `{% if %}` and `{% for %}`. A box is shown or hidden with `ifSt` ([step 7](#step-7-field-pipes-and-ifst)), not with a template tag.
- 1-bit Zebra preview, test print, ZIP export.
- Rewriting the screen as a non-Angular Vite app.
- Storing the page as PDF-tool YAML, and PDF preview, are not steps in this file. They are the gate in [yaml-and-preview-plan.md](yaml-and-preview-plan.md), and [step 6](#step-6-run) starts after that plan's Run. The field notes are [pdf-yaml-mapping.md](pdf-yaml-mapping.md).
