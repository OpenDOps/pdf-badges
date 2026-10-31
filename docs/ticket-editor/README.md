# Ticket editor

The badge layout editor that lives in `PIR_REG/expocab-offline-registration`. These notes describe that editor, how to run it without the registration node, how its boxes relate to the PDF tool's YAML page, and how to store that page and preview it.

| Document | What it is |
|---|---|
| [implementation-plan.md](implementation-plan.md) | Step-by-step plan: Angular editor, no operator login, small Node process for templates, assets, and demo data |
| [toolbar-contributions.md](toolbar-contributions.md) | How to add a print or preview toolbar back onto a route. [Step 5](implementation-plan.md#step-5-plug-toolbar-actions-into-the-route) points here |
| [yaml-and-preview-plan.md](yaml-and-preview-plan.md) | Gate before [implementation step 6](implementation-plan.md#step-6-run): store the page as PDF-tool YAML, run the form backend and pdf-tooling in Compose, and download a PDF preview |
| [steps-1-7-audit.md](steps-1-7-audit.md) | Logic and performance issues in steps 1–7 of that plan, ordered by severity |
| [editor.md](editor.md) | How the editor is built, the page, the fields, `formats()`, `demoContext()`, and the asset calls |
| [pdf-yaml-mapping.md](pdf-yaml-mapping.md) | Which boxes already match a PDF YAML template, and which YAML fields are still missing |
| [pir-template-import-plan.md](pir-template-import-plan.md) | Test plan: download one PIR badge ZIP, convert its HTML and CSS to `template.yml`, and open that page in the editor |
| [toolbox-plan.md](toolbox-plan.md) | Tool rail replaces the variable and asset pane: select, marquee, text, image, rectangle, QR, and barcode, plus the image dialog, variables in text, color pickers, and alignment |
| [tool-plugins.md](tool-plugins.md) | One sheet-tool object: rail button, cursor, and either a click or a shared corner drag |
| [tool-plugins-refactor.md](tool-plugins-refactor.md) | Move the current tools onto that object, one step at a time, with the test for each step |
| [tool-plugins-audit.md](tool-plugins-audit.md) | Logic gaps and performance issues after that refactor, ordered by severity |
| [typing-history.md](typing-history.md) | How undo should group keystrokes while a text box is open, and fold that visit into one page event when the operator leaves the text |

Source of the editor: `PIR_REG/expocab-offline-registration/frontend/src/app/operator/templates/`. It is TypeScript (Angular 20). The PDF page it is compared with is [document-schema.md](../pdf-tooling/document-schema.md).
