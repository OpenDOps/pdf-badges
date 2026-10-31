import { NgComponentOutlet } from '@angular/common';
import { Component, HostListener, OnDestroy, OnInit, ViewChild, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { ActivatedRoute, Router, RouterLink } from '@angular/router';
import { finalize, forkJoin, of } from 'rxjs';
import { catchError } from 'rxjs/operators';

import { demoContext as localDemoContext } from '../../../../../src/demo-context';
import { pageFormats } from '../../../../../src/formats';
import { environment } from '../../../environments/environment';
import { PrintApiService } from '../../core/api/print-api.service';
import { PublicApiService } from '../../core/api/public-api.service';
import {
  DemoContext,
  PageFormat,
  PageFormatsCatalog,
  PagePayload,
  PrintTemplateDetail,
  PrintTemplateWrite,
} from '../../core/models/print.models';
import { FormlySchema } from '../../core/models/registration.models';
import { ToastService } from '../../core/toast.service';
import { WorkstationPrinterService } from '../../core/workstation-printer.service';
import { CodeEditorComponent, CompletionItem } from './code-editor.component';
import { EDITOR_CONTRIBUTIONS, EDITOR_SHEET, EditorContribution, EditorSheet } from './editor-sheet';
import { PageDocument, PageEntry, PaletteEntry, appendMustache, appendPaletteEntry, applyEntryColor, applyPageJson, entryToHtml, formatPageJson, growEntryToText, pageToHtml, removeEntry, syncEntryFromCanvas } from './page-view';
import { PageHistory, changedEntryId, historyShortcut, pageHistoryCommand } from './page-history';
import { measureLineMm, normalizeHex, textStyleCache, type TextStyleMemory } from './text-style';
import { PaletteGroup, buildPalette, visualModeBlocker } from './template-snippets';
import { applyToolEffect } from './tools/apply-effect';
import { toolById } from './tools/registry';
import { entryFromTarget, type GestureInput, type ToolHost } from './tools/sheet-tool';
import { ColorPick, EditorTool, ToolRailComponent } from './tool-rail.component';
import { PaletteInsert } from './variable-palette.component';
import { VisualEditorComponent } from './visual-editor.component';

type Mode = 'visual' | 'code';

/** Редактор макета (docs/TZ_print_templates.md, раздел 6): один экран, два
 * режима — «Визуально» (GrapesJS) и «Код» (CodeMirror). Результат обоих —
 * один HTML-документ (`html`), который и печатается; project data GrapesJS
 * (`editor_state`) хранится только чтобы снова открыть визуальный режим.
 * Облачный макет открывается только для чтения. */
@Component({
  selector: 'app-template-editor',
  standalone: true,
  imports: [FormsModule, RouterLink, NgComponentOutlet, CodeEditorComponent, ToolRailComponent, VisualEditorComponent],
  templateUrl: './template-editor.component.html',
  providers: [{ provide: EDITOR_SHEET, useExisting: TemplateEditorComponent }],
  // The editor fills EditorLayoutComponent's <main> (a full-height flex
  // column) exactly: side panels scroll inside themselves instead of
  // growing the page.
  host: { class: 'flex flex-col flex-1 min-h-0' },
})
export class TemplateEditorComponent implements OnInit, OnDestroy, EditorSheet {
  private readonly route = inject(ActivatedRoute);
  private readonly router = inject(Router);
  private readonly api = inject(PrintApiService);
  private readonly publicApi = inject(PublicApiService);
  private readonly toast = inject(ToastService);
  readonly workstation = inject(WorkstationPrinterService);

  @ViewChild(CodeEditorComponent) codeEditor?: CodeEditorComponent;
  @ViewChild(VisualEditorComponent) visualEditor?: VisualEditorComponent;

  readonly template = signal<PrintTemplateDetail | null>(null);
  readonly loading = signal(true);
  readonly saving = signal(false);
  readonly dirty = signal(false);
  readonly mode = signal<Mode>('code');
  readonly visualBlocked = signal<string | null>(null);
  readonly errorMessage = signal<string | null>(null);
  readonly errorLine = signal<number | null>(null);
  readonly conflict = signal(false);

  /** `/editor/:id` reads the bundled catalog. `/operator/templates/:id` still fetches it. */
  readonly localCatalog = signal(this.route.snapshot.data['localCatalog'] === true);
  readonly formats = signal<PageFormatsCatalog | null>(null);
  readonly demo = signal<DemoContext | null>(null);
  readonly schema = signal<FormlySchema | null>(null);
  readonly palette = signal<PaletteGroup[]>([]);
  readonly completions = signal<CompletionItem[]>([]);
  readonly fileUrls = signal<Record<string, string>>({});

  readonly contributions: EditorContribution[] = inject(EDITOR_CONTRIBUTIONS, { optional: true }) ?? [];
  readonly zoom = signal(100);
  /** The sheet tool. Select leaves the canvas to move boxes. Later steps read this. */
  readonly tool = signal<EditorTool>('select');
  readonly swatches = signal({ foreground: textStyleCache.read().foreground, background: textStyleCache.read().background });
  private readonly selection = signal<string[]>([]);
  private colorReload: ReturnType<typeof setTimeout> | null = null;

  // Editable state (the form) — separate from the loaded row so dirty
  // detection and the optimistic-lock token stay simple.
  name = '';
  html = '';
  pageFormat: PageFormat = 'custom';
  widthMm: number | null = 100;
  heightMm: number | null = 100;
  orientation: 'portrait' | 'landscape' = 'portrait';
  margins = { top: 0, right: 0, bottom: 0, left: 0 };
  /** `/editor/:id` only. The operator build leaves this false. */
  readonly yamlPage = environment.yamlPage;
  bleeds = { top: 0, right: 0, bottom: 0, left: 0 };

  private loadedUpdatedAt = '';
  /** The page object from GET. Save sends this, and replaces it with the reply. */
  private pageDocument: PageDocument | null = null;
  /** JSON of the page last loaded or saved. Undo back to it clears the dirty flag. */
  private savedPageJson = '';
  /** Creations of a text box, a rectangle, a QR, or a barcode. The canvas stack is cleared when the sheet reloads. */
  private readonly pageHistory = new PageHistory();
  private objectUrls: string[] = [];
  /** What the visual editor should load once it reports `ready` — it is
   * created by an @if when the mode flips, so it isn't there to call yet. */
  private pendingVisual: { html: string; projectData?: unknown } | null = null;

  get isCloud(): boolean {
    return this.template()?.source === 'cloud';
  }

  get readonly(): boolean {
    return this.isCloud;
  }

  get pagePayload(): PagePayload {
    return {
      format: this.pageFormat,
      width_mm: this.pageFormat === 'custom' ? Number(this.widthMm) : null,
      height_mm: this.pageFormat === 'custom' ? Number(this.heightMm) : null,
      orientation: this.orientation,
      margins_mm: {
        top: Number(this.margins.top) || 0,
        right: Number(this.margins.right) || 0,
        bottom: Number(this.margins.bottom) || 0,
        left: Number(this.margins.left) || 0,
      },
    };
  }

  get printerDpi(): number {
    const printer = this.workstation.selected();
    const zebra =
      printer?.dispatch_mode === 'zpl_tcp' ||
      (printer?.dispatch_mode === 'agent' && printer.agent_kind === 'raw_zpl');
    return zebra && printer ? printer.dpi : 203;
  }

  ngOnInit(): void {
    window.addEventListener('keydown', this.onHistoryKey, true);
    const id = this.route.snapshot.paramMap.get('id') ?? '';
    if (this.localCatalog()) {
      this.formats.set(pageFormats);
      this.publishPalette(localDemoContext as unknown as DemoContext, null);
    } else {
      this.api.formats().subscribe({ next: (f) => this.formats.set(f), error: () => undefined });
    }
    this.api.getTemplate(id).subscribe({
      next: (template) => this.applyLoaded(template),
      error: () => {
        this.loading.set(false);
        if (this.localCatalog()) {
          this.errorMessage.set('Макет не найден.');
          return;
        }
        this.toast.error('Макет не найден.');
        this.router.navigate(['/operator/templates']);
      },
    });
  }

  ngOnDestroy(): void {
    window.removeEventListener('keydown', this.onHistoryKey, true);
    this.objectUrls.forEach((url) => URL.revokeObjectURL(url));
  }

  @HostListener('window:beforeunload', ['$event'])
  onBeforeUnload(event: BeforeUnloadEvent) {
    if (this.dirty()) {
      event.preventDefault();
    }
  }

  /** Route canDeactivate guard target (app.routes.ts). */
  canDeactivate(): boolean {
    return !this.dirty() || confirm('Есть несохранённые изменения. Уйти без сохранения?');
  }

  private applyLoaded(template: PrintTemplateDetail) {
    this.template.set(template);
    this.name = template.name;
    this.pageDocument = template.page ? (template.page as unknown as PageDocument) : null;
    this.html = this.pageDocument ? pageToHtml(this.pageDocument) : (template.html ?? '');
    this.pageFormat = template.page_format ?? 'custom';
    this.widthMm = template.page_width_mm === null ? null : Number(template.page_width_mm);
    this.heightMm = template.page_height_mm === null ? null : Number(template.page_height_mm);
    this.orientation = template.orientation ?? 'portrait';
    this.margins = {
      top: Number(template.margin_top_mm),
      right: Number(template.margin_right_mm),
      bottom: Number(template.margin_bottom_mm),
      left: Number(template.margin_left_mm),
    };
    const bleeds = this.pageDocument?.page_size.bleeds;
    this.bleeds = {
      top: Number(bleeds?.top) || 0,
      right: Number(bleeds?.right) || 0,
      bottom: Number(bleeds?.bottom) || 0,
      left: Number(bleeds?.left) || 0,
    };
    this.loadedUpdatedAt = template.updated_at;
    this.pageHistory.clear();
    this.rememberSaved();
    this.dirty.set(false);
    this.conflict.set(false);
    this.visualBlocked.set(visualModeBlocker(this.html));

    if (!this.localCatalog()) {
      forkJoin({
        demo: this.api.demoContext(template.ticket_type).pipe(catchError(() => of(null))),
        schema: this.publicApi.getFormlySchema(template.ticket_type).pipe(catchError(() => of(null))),
      }).subscribe(({ demo, schema }) => this.publishPalette(demo, schema));
    }

    this.loadFileUrls(template);
    this.loading.set(false);

    // Open visually when there's saved editor state and nothing blocks it;
    // a cloud or code-authored template opens in code mode.
    const startVisual = !this.isCloud && !this.visualBlocked() && (template.editor_state != null || this.pageDocument != null);
    if (startVisual) {
      this.pendingVisual = this.pageDocument
        ? { html: this.html }
        : { html: template.html ?? '', projectData: template.editor_state };
    }
    this.mode.set(startVisual ? 'visual' : 'code');
  }

  private publishPalette(demo: DemoContext | null, schema: FormlySchema | null): void {
    this.demo.set(demo);
    this.schema.set(schema);
    const groups = buildPalette(demo, schema);
    this.palette.set(groups);
    this.completions.set(
      groups
        .flatMap((g) =>
          g.items.map((i) => ({ label: i.expr, detail: `${i.label} — ${g.title}`, apply: `${i.expr} ` }))
        )
        .concat(
          ['barcode_svg', 'qrcode_svg', 'vcard_qrcode_svg', 'barcode', 'qrcode', 'vcard_qrcode'].map((v) => ({
            label: v,
            detail: 'картинка кода',
            apply: `${v} `,
          }))
        )
    );
    this.visualEditor?.refreshDemo();
  }

  onVisualReady() {
    const pending = this.pendingVisual;
    this.pendingVisual = null;
    if (!this.visualEditor) {
      return;
    }
    if (pending?.projectData != null) {
      this.visualEditor.loadProjectData(pending.projectData, pending.html);
    } else {
      this.visualEditor.loadHtml(pending?.html ?? this.html);
    }
    this.visualEditor.refreshDemo();
  }

  /** The canvas can't fetch `/api/.../assets/<name>` itself (auth header,
   * not a cookie), so files are pulled here into object URLs. */
  private loadFileUrls(template: PrintTemplateDetail) {
    const names = Object.keys(template.manifest ?? {});
    if (names.length === 0) {
      this.fileUrls.set({});
      return;
    }
    forkJoin(
      Object.fromEntries(
        names.map((name) => [name, this.api.getAsset(template.id, name).pipe(catchError(() => of(null)))])
      )
    ).subscribe((blobs) => {
      const urls: Record<string, string> = {};
      for (const [name, blob] of Object.entries(blobs)) {
        if (blob) {
          const url = URL.createObjectURL(blob as Blob);
          this.objectUrls.push(url);
          urls[name] = url;
        }
      }
      this.fileUrls.set(urls);
      this.visualEditor?.refreshDemo();
    });
  }

  // --- mode switching ---

  setMode(mode: Mode) {
    if (mode === this.mode()) {
      return;
    }
    if (mode === 'visual') {
      if (this.yamlPage && this.pageDocument) {
        const applied = applyPageJson(this.html);
        if (!applied.ok) {
          this.errorMessage.set(applied.detail);
          this.toast.error(applied.detail);
          return;
        }
        this.pageDocument = applied.page;
        this.errorMessage.set(null);
        this.html = pageToHtml(this.pageDocument);
        this.pendingVisual = { html: this.html };
        this.visualBlocked.set(null);
        this.mode.set('visual');
        return;
      }
      const blocker = visualModeBlocker(this.html);
      this.visualBlocked.set(blocker);
      if (blocker) {
        this.toast.error(blocker);
        return;
      }
      this.pendingVisual = { html: this.html };
      this.mode.set('visual');
    } else if (this.yamlPage && this.pageDocument) {
      // Keep the typed text on the page object before the canvas is torn down.
      this.visualEditor?.finishTextEdit();
      this.html = formatPageJson(this.pageDocument);
      this.mode.set('code');
    } else {
      this.visualEditor?.finishTextEdit();
      if (this.visualEditor) {
        this.html = this.visualEditor.exportDocument();
      }
      this.mode.set('code');
    }
  }

  onCodeChange(value: string) {
    if (value !== this.html) {
      this.html = value;
      this.markDirty();
    }
  }

  onVisualChange() {
    this.markDirty();
  }

  onPageChange() {
    this.markDirty();
    this.visualEditor?.setPage(this.pagePayload);
  }

  onPreset(value: string) {
    const [w, h] = value.split('x').map(Number);
    if (!w || !h) {
      return;
    }
    this.pageFormat = 'custom';
    this.widthMm = w;
    this.heightMm = h;
    this.onPageChange();
  }

  markDirty() {
    if (!this.readonly) {
      this.dirty.set(true);
    }
  }

  private rememberSaved(): void {
    this.savedPageJson = this.pageDocument ? formatPageJson(this.pageDocument) : '';
  }

  onZoom(value: number) {
    this.zoom.set(Number(value));
    this.visualEditor?.setZoom(Number(value));
  }

  fitToView() {
    this.visualEditor?.fitToView();
  }

  onYamlDrop(insert: PaletteEntry) {
    if (!this.yamlPage || !this.pageDocument || this.readonly) return;
    this.placeEntry(insert);
  }

  /** The active tool decides. An append records one history step. An edit opens the caret. */
  onToolGesture(input: GestureInput) {
    if (!this.yamlPage || !this.pageDocument || this.readonly) return;
    // One source of truth: the live page, not a mirrored canvas input.
    const entry = entryFromTarget(this.pageDocument, input.target);
    const effect = toolById(this.tool()).onGesture({ ...input, entry }, this.toolHost());
    const result = applyToolEffect(effect, this.pageDocument, this.pageHistory);
    if (result.kind === 'edit-text') {
      this.visualEditor?.beginTextEdit(result.id, result.client);
      return;
    }
    if (result.kind === 'set-foreground') {
      this.applyForegroundSample(result.hex);
      return;
    }
    if (result.kind === 'select-entries') {
      this.applyBandSelection(result.ids);
      return;
    }
    if (result.kind !== 'appended') return;
    this.insertAppended(result.id);
    this.markDirty();
  }

  /** Select box: canvas selection matches the ids (empty clears). Nothing is stored. */
  private applyBandSelection(ids: string[]) {
    this.selection.set([...ids]);
    this.visualEditor?.selectEntries(ids);
    const last = ids[ids.length - 1];
    if (last) {
      const entry = this.pageDocument?.contents.find((item) => item.id === last);
      this.rememberStyle(entry);
    }
  }

  /** Eyedropper: foreground memory and the swatch only. The page entry stays put. */
  private applyForegroundSample(hex: string) {
    const color = normalizeHex(hex);
    if (!color) return;
    const style = textStyleCache.read();
    if (style.foreground === color) {
      this.swatches.set({ foreground: style.foreground, background: style.background });
      return;
    }
    textStyleCache.remember({ ...style, foreground: color });
    this.swatches.set({ foreground: color, background: style.background });
  }

  /** Paint only the new box. A full sheet reload stays for undo, traits, and color. */
  private insertAppended(id: string) {
    if (!this.pageDocument) return;
    const entry = this.pageDocument.contents.find((item) => item.id === id);
    const html = entry ? entryToHtml(entry, this.pageDocument) : '';
    this.html = pageToHtml(this.pageDocument);
    if (html) this.visualEditor?.appendEntry(html, id);
    else this.showPage(id);
  }

  private toolHost(): ToolHost {
    return {
      page: () => this.pageDocument,
      style: () => this.usableStyle(textStyleCache.read()),
      measureLine: (font, sizeMm, bold, italic) => this.measureFace(font, sizeMm, bold, italic),
      editingElement: () => this.visualEditor?.editingElement() ?? null,
      placeGuard: () => this.visualEditor?.placeGuard() ?? false,
      beginTextEdit: (id, client) => this.visualEditor?.beginTextEdit(id, client),
      samplePicture: (id, point) => {
        const entry = this.pageDocument?.contents.find((item) => item.id === id);
        if (!entry) return null;
        return this.visualEditor?.samplePictureAt(entry, point) ?? null;
      },
      selection: () => this.selection(),
    };
  }

  /**
   * ⌘Z / ⌘⇧Z. One page stack, the same one the sheet uses. A text field keeps
   * the shortcut while it is focused.
   */
  onHistoryKey = (event: KeyboardEvent): void => {
    if (!this.yamlPage || this.readonly || !this.pageDocument) return;
    const fromCanvas = event.view !== window;
    const target = event.target as { tagName?: string; isContentEditable?: boolean } | null;
    const tag = target?.tagName;
    if (target?.isContentEditable) return;
    if (!fromCanvas && (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT')) return;
    if (!historyShortcut(event)) return;
    event.preventDefault();
    event.stopPropagation();
    const command = pageHistoryCommand(event, { undo: this.pageHistory.canUndo, redo: this.pageHistory.canRedo });
    if (!command) return;
    const current = this.pageDocument;
    const restored = command === 'undo' ? this.pageHistory.undo(current) : this.pageHistory.redo(current);
    if (!restored) return;
    const selectId = changedEntryId(current, restored);
    this.pageDocument = restored;
    this.showPage(selectId ?? undefined);
    this.selection.set(selectId ? [selectId] : []);
    this.dirty.set(formatPageJson(restored) !== this.savedPageJson);
  };

  /** The canvas already dropped the box. Forget it on the page, or the next add draws it again. */
  onEntryRemove(id: string) {
    if (!this.yamlPage || !this.pageDocument || this.readonly) return;
    if (!this.pageDocument.contents.some((entry) => entry.id === id)) return;
    this.pageHistory.record(this.pageDocument);
    removeEntry(this.pageDocument, id);
    this.selection.set(this.selection().filter((item) => item !== id));
    this.markDirty();
  }

  /** The selected text box becomes the face the next Text click uses. */
  onEntrySelect(id: string) {
    this.selection.set([id]);
    const entry = this.pageDocument?.contents.find((item) => item.id === id);
    this.rememberStyle(entry);
  }

  /** The rail already stored the hex. Selected entries receive it. The file is written only by Save. */
  onColorPick(pick: ColorPick) {
    this.swatches.set({ foreground: textStyleCache.read().foreground, background: textStyleCache.read().background });
    if (!this.pageDocument) return;
    const ids = this.selection();
    let changed = false;
    for (const id of ids) {
      const entry = this.pageDocument.contents.find((item) => item.id === id);
      if (entry && applyEntryColor(entry, pick.target, pick.hex)) changed = true;
    }
    if (!changed) return;
    this.markDirty();
    if (this.colorReload) clearTimeout(this.colorReload);
    this.colorReload = setTimeout(() => this.showPage(ids[0]), 40);
  }

  onBoxChange(event: { id: string; style: Record<string, string>; attrs: Record<string, string>; source: 'trait' | 'geometry' }) {
    if (!this.pageDocument) return;
    const entry = this.pageDocument.contents.find((item) => item.id === event.id);
    if (!entry) return;
    const beforePage = structuredClone(this.pageDocument);
    const before = this.styleSnapshot(entry);
    syncEntryFromCanvas(entry, event, event.source);
    this.rememberStyle(entry);
    const grew = this.refitText(entry, before);
    if (formatPageJson(this.pageDocument) === formatPageJson(beforePage)) return;
    this.pageHistory.record(beforePage);
    this.markDirty();
    if (grew || (event.source === 'trait' && pageToHtml(this.pageDocument) !== this.html)) {
      this.showPage(event.id);
    }
  }

  onPreenteredEdit(event: { id: string; value: string }) {
    if (!this.pageDocument) return;
    const entry = this.pageDocument.contents.find((item) => item.id === event.id);
    if (!entry || entry.preentered === event.value) return;
    this.pageHistory.record(this.pageDocument);
    entry.preentered = event.value;
    entry.content = null;
    if (/\{\{/.test(event.value)) entry.template = true;
    else delete entry.template;
    if (entry.text) growEntryToText(entry, this.measureFace(entry.text.font, entry.text.font_size, !!entry.text.font_style?.bold, !!entry.text.font_style?.italic));
    this.showPage(entry.id);
    this.markDirty();
  }

  onBleedsChange() {
    if (!this.pageDocument) return;
    this.pageDocument.page_size = {
      ...this.pageDocument.page_size,
      bleeds: {
        top: Number(this.bleeds.top) || 0,
        right: Number(this.bleeds.right) || 0,
        bottom: Number(this.bleeds.bottom) || 0,
        left: Number(this.bleeds.left) || 0,
      },
    };
    this.markDirty();
    this.visualEditor?.setBleeds(this.pageDocument.page_size.bleeds!);
  }

  fontChoices(): { id: string; label: string }[] {
    const fonts = this.pageDocument?.resources.fonts ?? {};
    return Object.entries(fonts).map(([id, font]) => ({
      id,
      label: id === 'text' ? 'Liberation Serif' : font.font_family || id,
    }));
  }

  fontFaces(): { family: string; url: string }[] {
    const fonts = this.pageDocument?.resources.fonts ?? {};
    const urls = this.fileUrls();
    const faces: { family: string; url: string }[] = [];
    for (const [key, font] of Object.entries(fonts)) {
      const family = font.font_family;
      const url = urls[key] ?? (family ? urls[family] : undefined);
      if (family && url) faces.push({ family, url });
    }
    return faces;
  }

  // --- save ---

  page(): PagePayload {
    return this.pagePayload;
  }

  templateId(): number | null {
    return this.template()?.id ?? null;
  }

  templateName(): string {
    return this.template()?.name ?? this.name;
  }

  unsaved(): boolean {
    return this.dirty();
  }

  showError(message: string | null, line?: number | null): void {
    this.errorMessage.set(message);
    this.errorLine.set(line ?? null);
  }

  /** Toolbar millimetres win for a custom sheet. Contents, including preentered, stay as loaded. */
  private pageForSave(): PageDocument | null {
    if (!this.pageDocument) return null;
    const page = structuredClone(this.pageDocument);
    if (this.pageFormat === 'custom' && this.widthMm != null && this.heightMm != null) {
      page.page_size = {
        ...page.page_size,
        width: Number(this.widthMm),
        height: Number(this.heightMm),
        units: page.page_size.units ?? 'mm',
      };
    }
    return page;
  }

  currentHtml(): string {
    if (this.yamlPage && this.pageDocument) return pageToHtml(this.pageDocument);
    if (this.mode() === 'visual' && this.visualEditor) {
      this.html = this.visualEditor.exportDocument();
    }
    return this.html;
  }

  save() {
    const template = this.template();
    if (!template || this.saving() || this.readonly) {
      return;
    }
    if (this.yamlPage && this.pageDocument && this.mode() === 'code') {
      const applied = applyPageJson(this.html);
      if (!applied.ok) {
        this.errorMessage.set(applied.detail);
        this.toast.error(applied.detail);
        return;
      }
      this.pageDocument = applied.page;
      this.errorMessage.set(null);
    }
    this.saving.set(true);
    this.errorMessage.set(null);
    this.errorLine.set(null);
    const page = this.pagePayload;
    const document = this.pageForSave();
    const payload: PrintTemplateWrite = {
      name: this.name.trim() || template.name,
      html: document ? undefined : this.currentHtml(),
      page: document ? (document as unknown as PrintTemplateWrite['page']) : undefined,
      editor_state: document ? null : this.mode() === 'visual' ? this.visualEditor?.projectData() ?? null : null,
      page_format: page.format,
      page_width_mm: page.width_mm,
      page_height_mm: page.height_mm,
      orientation: page.orientation,
      margin_top_mm: page.margins_mm.top,
      margin_right_mm: page.margins_mm.right,
      margin_bottom_mm: page.margins_mm.bottom,
      margin_left_mm: page.margins_mm.left,
      updated_at: this.loadedUpdatedAt,
    };
    this.api
      .updateTemplate(template.id, payload)
      .pipe(finalize(() => this.saving.set(false)))
      .subscribe({
        next: (saved) => {
          this.template.set(saved);
          if (saved.page) {
            this.pageDocument = saved.page as unknown as PageDocument;
            const bleeds = this.pageDocument.page_size.bleeds;
            this.bleeds = {
              top: Number(bleeds?.top) || 0,
              right: Number(bleeds?.right) || 0,
              bottom: Number(bleeds?.bottom) || 0,
              left: Number(bleeds?.left) || 0,
            };
            this.html = this.mode() === 'code' ? formatPageJson(this.pageDocument) : pageToHtml(this.pageDocument);
            if (this.mode() === 'visual') this.visualEditor?.reloadDocument(this.html);
            this.widthMm = saved.page_width_mm === null ? null : Number(saved.page_width_mm);
            this.heightMm = saved.page_height_mm === null ? null : Number(saved.page_height_mm);
          }
          this.loadedUpdatedAt = saved.updated_at;
          this.rememberSaved();
          this.dirty.set(false);
          this.conflict.set(false);
          this.toast.success('Макет сохранён.');
        },
        error: (err) => {
          if (err?.status === 409) {
            this.conflict.set(true);
          }
          this.errorMessage.set(err?.error?.detail ?? this.firstFieldError(err) ?? 'Не удалось сохранить макет.');
          const line = err?.error?.line;
          this.errorLine.set(typeof line === 'number' ? line : null);
          if (typeof line === 'number' && this.mode() === 'visual') {
            this.setMode('code');
          }
        },
      });
  }

  reload() {
    const template = this.template();
    if (!template) {
      return;
    }
    this.dirty.set(false);
    this.api.getTemplate(template.id).subscribe((fresh) => {
      const wasVisual = this.mode() === 'visual';
      this.mode.set('code'); // tear the canvas down so it reloads cleanly
      this.applyLoaded(fresh);
      if (wasVisual && !this.visualBlocked()) {
        this.pendingVisual = this.pageDocument
          ? { html: this.html }
          : { html: fresh.html ?? '', projectData: fresh.editor_state };
        this.mode.set('visual');
      }
    });
  }

  /** A palette click on a draggable row can also emit a drop. Collapse that pair. */
  private lastInsertKey = '';
  private lastInsertAt = 0;

  private placeEntry(insert: PaletteInsert | PaletteEntry) {
    const page = this.pageDocument;
    if (!page || !insert.pageKind) return;
    const selectedId = this.visualEditor?.selectedEntryId();
    const selected = selectedId ? page.contents.find((item) => item.id === selectedId) : undefined;
    const key = `${insert.pageKind}:${insert.editorPath ?? ''}:${insert.codeKind ?? ''}:${insert.fileName ?? ''}`;
    const now = Date.now();
    if (key === this.lastInsertKey && now - this.lastInsertAt < 500) return;
    this.lastInsertKey = key;
    this.lastInsertAt = now;
    this.pageHistory.record(page);
    if (insert.pageKind === 'var' && insert.editorPath && selected?.text) {
      appendMustache(selected, insert.editorPath);
      this.showPage(selected.id);
      this.markDirty();
      return;
    }
    const entry = appendPaletteEntry(page, { ...insert, pageKind: insert.pageKind });
    this.showPage(entry.id);
    this.markDirty();
  }

  private usableStyle(style: TextStyleMemory): TextStyleMemory {
    const font = this.pageDocument?.resources.fonts?.[style.font] ? style.font : 'text';
    return { ...style, font };
  }

  private measureFace(font: string, fontSizeMm: number, bold: boolean, italic: boolean): (line: string) => number {
    const family = this.pageDocument?.resources.fonts?.[font]?.font_family || 'Liberation Serif';
    const doc = this.visualEditor?.canvasDocument() ?? document;
    return (line) => measureLineMm(line, family, fontSizeMm, bold, italic, doc);
  }

  private rememberStyle(entry: PageEntry | undefined) {
    if (!entry) return;
    const current = textStyleCache.read();
    const text = entry.text;
    textStyleCache.remember({
      font: text?.font || current.font,
      fontSizePt: text ? Math.round(((text.font_size * 72) / 25.4) * 100) / 100 : current.fontSizePt,
      bold: text ? !!text.font_style?.bold : current.bold,
      italic: text ? !!text.font_style?.italic : current.italic,
      underline: text ? !!text.font_style?.underline : current.underline,
      foreground: text?.color && normalizeHex(text.color) ? normalizeHex(text.color)! : text ? '#000000' : current.foreground,
      background: entry.background && normalizeHex(entry.background) ? normalizeHex(entry.background)! : current.background,
    });
    this.swatches.set({ foreground: textStyleCache.read().foreground, background: textStyleCache.read().background });
  }

  private styleSnapshot(entry: PageEntry) {
    const text = entry.text;
    return {
      font: text?.font ?? '',
      font_size: text?.font_size ?? 0,
      bold: !!text?.font_style?.bold,
      italic: !!text?.font_style?.italic,
      underline: !!text?.font_style?.underline,
      pre: entry.preentered ?? '',
      auto: !!entry.auto_scale,
    };
  }

  /** A font or text change extends the frame. A drag does not. */
  private refitText(entry: PageEntry, before: ReturnType<TemplateEditorComponent['styleSnapshot']>): boolean {
    if (!entry.text || entry.auto_scale) return false;
    const text = entry.text;
    const same =
      before.font === text.font &&
      before.font_size === text.font_size &&
      before.bold === !!text.font_style?.bold &&
      before.italic === !!text.font_style?.italic &&
      before.underline === !!text.font_style?.underline &&
      before.pre === (entry.preentered ?? '') &&
      before.auto === !!entry.auto_scale;
    if (same) return false;
    return growEntryToText(entry, this.measureFace(text.font, text.font_size, !!text.font_style?.bold, !!text.font_style?.italic));
  }

  private showPage(selectId?: string) {
    if (!this.pageDocument) return;
    this.html = pageToHtml(this.pageDocument);
    this.visualEditor?.reloadDocument(this.html, selectId);
  }

  private firstFieldError(err: unknown): string | null {
    const body = (err as { error?: Record<string, unknown> })?.error;
    if (!body || typeof body !== 'object') {
      return null;
    }
    const entry = Object.entries(body).find(([k]) => k !== 'detail' && k !== 'line');
    if (!entry) {
      return null;
    }
    const [field, messages] = entry;
    return `${field}: ${Array.isArray(messages) ? messages.join(', ') : String(messages)}`;
  }

  createCopy() {
    const template = this.template();
    if (!template) {
      return;
    }
    const name = prompt('Название копии', `${template.name} (копия)`);
    if (!name) {
      return;
    }
    this.api.createTemplate({ ticket_type: template.ticket_type, name, copy_from: template.id }).subscribe({
      next: (created) => this.router.navigate(['/operator/templates', created.id]),
      error: () => this.toast.error('Не удалось создать копию.'),
    });
  }
}
