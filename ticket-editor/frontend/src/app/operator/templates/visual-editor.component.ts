import {
  AfterViewInit,
  Component,
  ElementRef,
  EventEmitter,
  Input,
  NgZone,
  OnChanges,
  OnDestroy,
  Output,
  SimpleChanges,
  ViewChild,
  inject,
} from '@angular/core';
import { createGesture, pinchAction, wheelAction, type FullGestureState } from '@use-gesture/vanilla';
import grapesjs, {
  Component as GjsComponent,
  ComponentResizeEventMoveProps,
  ComponentResizeEventStartProps,
  ComponentResizeEventUpdateProps,
  Editor,
} from 'grapesjs';
import ru from 'grapesjs/locale/ru.mjs';

import { environment } from '../../../environments/environment';
import { DemoContext, PagePayload } from '../../core/models/print.models';
import type { DrawnRect, SheetPoint } from './draw-tool';
import { PaletteEntry, operatorStyleSectors, yamlStyleSectors, type PageEntry } from './page-view';
import { DrawPointerSession, type DrawMode } from './pointer-session';
import { toolById } from './tools/registry';
import { picturePixel, rgbaToHex } from './tools/sample-color';
import type { GestureInput, PointerHost } from './tools/sheet-tool';
import { TextEditSession } from './tools/text-edit';
import { EditorTool } from './tool-rail.component';
import { ViewportPose, viewportAfterPan, viewportAfterPinch } from './viewport-gesture';
import {
  CODE_VARIABLES,
  CodeKind,
  PaletteGroup,
  CODE_DOTS,
  codeWidthMm,
  snapCodeWidthMm,
  demoValueFor,
  joinDocument,
  splitDocument,
  variableExpression,
} from './template-snippets';

const GRAPES_CSS = 'assets/grapesjs/grapes.min.css';
const PX_PER_MM = 96 / 25.4;
const GRID_MM = 0.5;
/** Grey desk around the sheet inside the frame, mm. */
const DESK_PAD_MM = 8;
/** The canvas colour in both layers (GrapesJS canvas and the frame's html),
 * so there is one desk, not two shades of grey. */
const DESK_COLOR = '#e5e7eb';
const ZOOM_MIN = 25;
const ZOOM_MAX = 400;
/** Everything a read-only (cloud) template must not allow on the canvas. */
const LOCKED_PROPS = {
  draggable: false,
  droppable: false,
  editable: false,
  removable: false,
  copyable: false,
  resizable: false,
  stylable: false,
};

/** Ruler thickness, CSS px. */
const RULER_PX = 22;

function formatMm(mm: number): string {
  const rounded = Math.round(mm * 10) / 10;
  return Number.isInteger(rounded) ? String(rounded) : rounded.toFixed(1);
}

function pxToMm(px: number): number {
  return Math.round((px / PX_PER_MM) / GRID_MM) * GRID_MM;
}

/** A length from a component style, in millimetres. `px` is what GrapesJS
 * writes while a box is being dragged; `mm` is what the sheet stores. */
function lengthMm(value: string | undefined): number | null {
  if (!value || !value.trim()) return null;
  const n = parseFloat(value);
  if (!Number.isFinite(n)) return null;
  if (/mm\s*$/.test(value)) return n;
  if (/cm\s*$/.test(value)) return n * 10;
  if (/in\s*$/.test(value)) return n * 25.4;
  if (/pt\s*$/.test(value)) return (n * 25.4) / 72;
  return n / PX_PER_MM;
}

/** Sheet edges of a canvas box. Style is preferred; the painted element is
 * the fallback for a node that has no box of its own. */
function boxMm(
  component: GjsComponent,
  origin: { x: number; y: number },
): { left: number; right: number; top: number; bottom: number } | null {
  const style = (component.getStyle?.() ?? {}) as Record<string, string>;
  const left = lengthMm(style['left']);
  const top = lengthMm(style['top']);
  const width = lengthMm(style['width']);
  const height = lengthMm(style['height']);
  if (left != null && top != null && width != null && height != null && width > 0 && height > 0) {
    return { left, right: left + width, top, bottom: top + height };
  }
  const el = component.getEl?.();
  if (!el) return null;
  const rect = el.getBoundingClientRect();
  if (rect.width <= 0 || rect.height <= 0) return null;
  const x = rect.left / PX_PER_MM - origin.x;
  const y = rect.top / PX_PER_MM - origin.y;
  return { left: x, right: x + rect.width / PX_PER_MM, top: y, bottom: y + rect.height / PX_PER_MM };
}

/** `14.5mm` → px; px (or anything unitless) passes through as a number. */
function lengthToPx(value: string | undefined): number {
  if (!value) {
    return 0;
  }
  const n = parseFloat(value);
  if (Number.isNaN(n)) {
    return 0;
  }
  if (/mm\s*$/.test(value)) return n * PX_PER_MM;
  if (/cm\s*$/.test(value)) return n * 10 * PX_PER_MM;
  if (/in\s*$/.test(value)) return n * 96;
  if (/pt\s*$/.test(value)) return (n * 96) / 72;
  return n;
}

function escapeAttr(value: string): string {
  return value.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;').replace(/\{/g, '&#123;');
}

/** GrapesJS wrapper — the visual mode of the template editor (docs/
 * TZ_print_templates.md, разделы 6.2, 6.4, 6.6). The canvas is the page in
 * mm (free placement, 0.5 mm snapping), the "smart" blocks are custom
 * component types that show demo values on the canvas and export Django
 * expressions:
 *   - tpl-var  (`data-var`)  → `{{ form_data.field_12|… }}`
 *   - tpl-code (`data-code`) → `<img src="{{ barcode_svg }}">`, sized in printer dots
 *   - tpl-file (`data-file`) → `<img src="{{ files.logo }}">`
 * Source of truth stays the exported HTML (exportDocument()); the GrapesJS
 * project data is only kept to reopen the visual mode losslessly. */
@Component({
  selector: 'app-visual-editor',
  standalone: true,
  // data-tool is for e2e only. The canvas cursor uses data-draw-cursor in the frame.
  host: { '[attr.data-tool]': 'tool' },
  template: `
    <div class="flex h-full min-h-0 gap-3">
      <div #desk class="relative flex-1 min-w-0 rounded-lg border border-border-grey-light overflow-hidden" [style.background]="deskColor">
        <div class="absolute inset-0" [style.left.px]="rulerPx" [style.top.px]="rulerPx">
          <div #canvas class="h-full w-full"></div>
        </div>
        <canvas #rulerX class="absolute top-0 left-0 w-full pointer-events-none" [style.height.px]="rulerPx"></canvas>
        <canvas #rulerY class="absolute left-0 h-full pointer-events-none" [style.top.px]="rulerPx" [style.width.px]="rulerPx"></canvas>
      </div>
      <div class="w-[300px] shrink-0 min-h-0 flex flex-col gap-2 overflow-y-auto text-sm">
        <div class="rounded-lg border border-border-grey-light bg-bg-white">
          <p class="px-3 py-1.5 text-xs uppercase tracking-wide text-text-secondary border-b border-border-grey-light">Свойства</p>
          <div #traits></div>
        </div>
        <div class="rounded-lg border border-border-grey-light bg-bg-white">
          <p class="px-3 py-1.5 text-xs uppercase tracking-wide text-text-secondary border-b border-border-grey-light">Стиль</p>
          <div #styles></div>
        </div>
        <div class="rounded-lg border border-border-grey-light bg-bg-white">
          <p class="px-3 py-1.5 text-xs uppercase tracking-wide text-text-secondary border-b border-border-grey-light">Слои</p>
          <div #layers></div>
        </div>
      </div>
    </div>
  `,
})
export class VisualEditorComponent implements AfterViewInit, OnChanges, OnDestroy {
  @ViewChild('canvas', { static: true }) canvasEl!: ElementRef<HTMLDivElement>;
  @ViewChild('desk', { static: true }) deskEl!: ElementRef<HTMLDivElement>;
  @ViewChild('rulerX', { static: true }) rulerXEl!: ElementRef<HTMLCanvasElement>;
  @ViewChild('rulerY', { static: true }) rulerYEl!: ElementRef<HTMLCanvasElement>;
  @ViewChild('traits', { static: true }) traitsEl!: ElementRef<HTMLDivElement>;
  @ViewChild('styles', { static: true }) stylesEl!: ElementRef<HTMLDivElement>;
  @ViewChild('layers', { static: true }) layersEl!: ElementRef<HTMLDivElement>;

  @Input() demo: DemoContext | null = null;
  @Input() groups: PaletteGroup[] = [];
  @Input() dpi = 203;
  /** name → object URL of the template's files, for the canvas only. */
  @Input() fileUrls: Record<string, string> = {};
  @Input() page: PagePayload = {
    format: 'custom', width_mm: 100, height_mm: 100, orientation: 'portrait',
    margins_mm: { top: 0, right: 0, bottom: 0, left: 0 },
  };
  /** Cloud templates open on the canvas for inspection only: nothing can be
   * moved, typed into, styled or removed. */
  @Input() readonly = false;
  /** `/editor/:id` writes the page object. The operator route leaves this off. */
  @Input() yamlPage = false;
  @Input() bleeds: { top: number; right: number; bottom: number; left: number } | null = null;
  /** Canvas-only faces. The stored picture does not point at these URLs. */
  @Input() fontFaces: { family: string; url: string }[] = [];
  /** Resource keys for the font trait. `text` is always Liberation Serif. */
  @Input() fonts: { id: string; label: string }[] = [];
  /** Sheet tool. Select leaves GrapesJS to move boxes. Any other tool takes the pointer. */
  @Input() tool: EditorTool = 'select';

  @Output() changed = new EventEmitter<void>();
  @Output() ready = new EventEmitter<void>();
  @Output() boxChange = new EventEmitter<{ id: string; style: Record<string, string>; attrs: Record<string, string>; source: 'trait' | 'geometry' }>();
  @Output() yamlDrop = new EventEmitter<PaletteEntry>();
  @Output() preenteredEdit = new EventEmitter<{ id: string; value: string }>();
  /** A finished click or drag. The editor runs the active tool and applies the effect. */
  @Output() toolGesture = new EventEmitter<GestureInput>();
  /** A key pressed in the canvas document. The sheet does not bubble keys to the parent. */
  @Output() historyKey = new EventEmitter<KeyboardEvent>();
  /** A box removed from the sheet, other than a reload or a drag. */
  @Output() entryRemove = new EventEmitter<string>();
  @Output() entrySelect = new EventEmitter<string>();
  /** The canvas re-fits the sheet into view on load and on page change;
   * the shell mirrors the resulting zoom into its slider. */
  @Output() zoomChanged = new EventEmitter<number>();

  private readonly zone = inject(NgZone);
  readonly deskColor = DESK_COLOR;
  readonly rulerPx = RULER_PX;
  private editor: Editor | null = null;
  private cssLink: HTMLLinkElement | null = null;
  private frameStyle: HTMLStyleElement | null = null;
  /** True between drag/resize start and end: GrapesJS writes px on every
   * move, and converting to mm mid-gesture would fight with it. */
  private gesture = false;
  /** Where the element was when a resize gesture started: its exact mm
   * offsets from the model plus the frame-px rect GrapesJS measured. */
  private resizeStart: {
    leftMm: number;
    topMm: number;
    l: number;
    t: number;
    h: number;
    /** Start font size in px for text-like blocks, 0 when the font must not follow the box. */
    fontPx: number;
    /** For barcode/QR blocks: the component and kind, so the width snaps to whole dots per module. */
    code: { component: GjsComponent; kind: 'barcode' | 'qrcode' } | null;
  } | null = null;
  /** Component just dropped from the palette, positioned on `canvas:dragend`. */
  private dropped: GjsComponent | null = null;
  private rulerRaf = 0;
  private pointerMm: { x: number; y: number } | null = null;
  /** The box under the pointer. Selection is used when nothing is hovered. */
  private hovered: GjsComponent | null = null;
  private resizeObserver: ResizeObserver | null = null;
  /** Pending content to load once the editor is up. */
  private pending: { html?: string; projectData?: unknown } | null = null;
  /** Set while setComponents runs, so a load does not write the page object back. */
  private loadingDoc = false;
  private pendingYamlInsert: string | null = null;
  /** Survives a palette click, which takes focus off the canvas. Cleared when the sheet itself is selected. */
  private lastYamlId: string | null = null;
  /** True while Select box drives Grapes selection so `entrySelect` does not collapse to one id. */
  private selectingBand = false;
  /** Pinch and wheel listeners on the current frame document. */
  private viewportGesture: { destroy(): void } | null = null;
  /** Pending zoom/pan restore after a full sheet reload. Cleared before a new one is scheduled. */
  private viewportRaf = 0;
  private viewportTimer: ReturnType<typeof setTimeout> | null = null;
  /** Pointer listeners for every tool except Select. The frame document owns them. */
  private drawSession: DrawPointerSession | null = null;
  /** Double-click listener on the current frame document. */
  private preenteredAbort: AbortController | null = null;
  /** Type in the box itself. The text tool opens this session. */
  private readonly textEdit = new TextEditSession((edit) => {
    this.zone.run(() => this.preenteredEdit.emit(edit));
  });

  ngAfterViewInit(): void {
    this.ensureCss();
    this.zone.runOutsideAngular(() => this.init());
  }

  /** The shell's `refreshDemo()` runs right after it sets its signals, before
   * these inputs are updated, so it would repaint with stale (null) demo
   * data. Repainting here sees the new inputs. */
  ngOnChanges(changes: SimpleChanges): void {
    if (changes['demo'] || changes['fileUrls'] || changes['groups']) {
      this.refreshDemo();
    }
    if (changes['bleeds'] || changes['fontFaces']) {
      this.applyPage();
      this.updateFrameStyle();
    }
    if (changes['fonts']) {
      this.editor?.getSelected()?.getTrait?.('data-font')?.set('options', this.fontOptions());
    }
    if (changes['tool']) {
      // A rail click does not blur the box. Keep the typed text before the new gesture takes the pointer.
      this.textEdit.finish();
      this.scheduleRulers();
      this.drawSession?.setTool(this.drawMode());
      this.updateFrameStyle();
    }
  }

  ngOnDestroy(): void {
    cancelAnimationFrame(this.rulerRaf);
    this.clearViewportKeep();
    this.resizeObserver?.disconnect();
    this.viewportGesture?.destroy();
    this.viewportGesture = null;
    this.drawSession?.dispose();
    this.drawSession = null;
    this.historyDoc?.removeEventListener('keydown', this.onCanvasHistoryKey, true);
    this.historyDoc = null;
    this.preenteredAbort?.abort();
    this.preenteredAbort = null;
    this.textEdit.dispose();
    this.editor?.destroy();
    this.editor = null;
  }

  // --- public API for the shell ---

  loadHtml(html: string, options?: { fit?: boolean }) {
    if (!this.editor) {
      this.pending = { html };
      return;
    }
    const { css, body } = splitDocument(html);
    // CSS first: parsing the markup turns inline `style=""` into rules, and
    // setStyle() would wipe those (every absolutely positioned element of a
    // hand-written template then falls back into the flow).
    this.loadingDoc = true;
    this.editor.setStyle(css);
    this.editor.setComponents(body);
    this.afterLoad(options?.fit !== false);
    setTimeout(() => (this.loadingDoc = false));
  }

  /** Replace the picture after the page object changed, and keep the same box selected.
   * Moving, typing, and the other edits leave the zoom and pan alone. */
  reloadDocument(html: string, selectId?: string) {
    const viewport = this.readViewport();
    this.loadHtml(html, { fit: false });
    if (!this.editor) return;
    if (selectId) {
      const found = this.editor.getWrapper()?.find(`[data-yaml-id="${selectId}"]`)?.[0];
      this.lastYamlId = selectId;
      if (found) this.editor.select(found);
    } else {
      this.lastYamlId = null;
    }
    // The page history owns undo. A selection must not become its own step.
    this.editor.UndoManager.clear();
    if (viewport) this.keepViewport(viewport);
  }

  /**
   * Add one box without rebuilding the sheet. Tool appends use this so zoom and
   * pan stay put and a 400 ms viewport restore never fights the next gesture.
   */
  appendEntry(blockHtml: string, selectId?: string): void {
    if (!this.editor || !blockHtml) return;
    this.clearViewportKeep();
    this.loadingDoc = true;
    const added = this.editor.addComponents(blockHtml);
    const last = Array.isArray(added) ? added[added.length - 1] : added;
    if (selectId) this.lastYamlId = selectId;
    if (last) this.editor.select(last as GjsComponent);
    this.editor.UndoManager.clear();
    setTimeout(() => (this.loadingDoc = false));
  }

  /**
   * Select box: canvas selection becomes these ids. Empty clears. Does not emit
   * `entrySelect` for each pick (the template editor already owns the list).
   */
  selectEntries(ids: string[]): void {
    if (!this.editor) return;
    const root = this.editor.getWrapper();
    const found = ids
      .map((id) => root?.find(`[data-yaml-id="${id}"]`)?.[0])
      .filter((component): component is GjsComponent => !!component);
    this.selectingBand = true;
    try {
      if (!found.length) {
        this.editor.select();
        this.lastYamlId = null;
        return;
      }
      this.editor.select(found[0]);
      for (let i = 1; i < found.length; i++) this.editor.selectAdd(found[i]);
      this.lastYamlId = ids[ids.length - 1] ?? null;
    } finally {
      this.selectingBand = false;
    }
  }

  /**
   * Eyedropper: `#RRGGBB` from the painted picture at a sheet point.
   * Null when the element is not an image, the pixel is transparent, or the canvas is tainted.
   */
  samplePictureAt(entry: PageEntry, point: SheetPoint): string | null {
    const doc = this.canvasDocument();
    if (!doc) return null;
    const el = doc.querySelector(`[data-yaml-id="${CSS.escape(entry.id)}"]`);
    if (!(el instanceof HTMLImageElement) || !el.naturalWidth || !el.naturalHeight) return null;
    const pixel = picturePixel(entry, point, el.naturalWidth, el.naturalHeight);
    if (!pixel) return null;
    const canvas = doc.createElement('canvas');
    canvas.width = el.naturalWidth;
    canvas.height = el.naturalHeight;
    const ctx = canvas.getContext('2d', { willReadFrequently: true });
    if (!ctx) return null;
    try {
      ctx.drawImage(el, 0, 0);
      const data = ctx.getImageData(pixel.x, pixel.y, 1, 1).data;
      return rgbaToHex(data[0], data[1], data[2], data[3]);
    } catch {
      return null;
    }
  }

  /** Zoom percent and pan, so a sheet reload can put the same view back. */
  private readViewport(): { zoom: number; x: number; y: number } | null {
    const canvas = this.editor?.Canvas;
    if (!canvas) return null;
    const coords = canvas.getCoords();
    return { zoom: canvas.getZoom(), x: coords.x, y: coords.y };
  }

  private clearViewportKeep(): void {
    if (this.viewportRaf) {
      cancelAnimationFrame(this.viewportRaf);
      this.viewportRaf = 0;
    }
    if (this.viewportTimer != null) {
      clearTimeout(this.viewportTimer);
      this.viewportTimer = null;
    }
  }

  /** The frame settles after a reload. Cancel a prior restore, then put zoom and pan back once. */
  private keepViewport(viewport: { zoom: number; x: number; y: number }): void {
    const editor = this.editor;
    if (!editor) return;
    this.clearViewportKeep();
    const apply = () => {
      if (this.editor !== editor) return;
      editor.Canvas.setZoom(viewport.zoom);
      editor.Canvas.setCoords(viewport.x, viewport.y);
      this.zone.run(() => this.zoomChanged.emit(Math.round(viewport.zoom)));
    };
    apply();
    this.viewportRaf = requestAnimationFrame(() => {
      this.viewportRaf = 0;
      apply();
    });
  }

  canvasDocument(): Document | null {
    return this.editor?.Canvas.getDocument() ?? null;
  }

  selectedEntryId(): string | null {
    const selected = this.editor?.getSelected();
    if (selected && selected !== this.editor?.getWrapper()) {
      const id = selected.getAttributes()?.['data-yaml-id'];
      if (typeof id === 'string' && id) return id;
    }
    if (!selected) return this.lastYamlId;
    return null;
  }

  setBleeds(bleeds: { top: number; right: number; bottom: number; left: number }) {
    this.bleeds = bleeds;
    this.applyPage();
    this.updateFrameStyle();
  }

  loadProjectData(data: unknown, fallbackHtml: string) {
    if (!this.editor) {
      this.pending = { projectData: data, html: fallbackHtml };
      return;
    }
    try {
      this.loadingDoc = true;
      this.editor.loadProjectData(data as Parameters<Editor['loadProjectData']>[0]);
      this.afterLoad();
      setTimeout(() => (this.loadingDoc = false));
    } catch {
      this.loadingDoc = false;
      this.loadHtml(fallbackHtml);
    }
  }

  private afterLoad(fit = true) {
    const editor = this.editor;
    if (!editor) {
      return;
    }
    this.applyPage();
    this.lockIfReadonly();
    editor.UndoManager.clear();
    if (fit) this.fitToView();
    else this.scheduleRulers();
  }

  exportDocument(): string {
    if (!this.editor) {
      return '';
    }
    const bodyHtml = this.editor.getHtml();
    const inner = bodyHtml.replace(/^[\s\S]*?<body[^>]*>/i, '').replace(/<\/body>[\s\S]*$/i, '');
    return joinDocument(this.editor.getCss() ?? '', inner);
  }

  projectData(): unknown {
    return this.editor?.getProjectData() ?? null;
  }

  setPage(page: PagePayload) {
    this.page = page;
    this.applyPage();
    this.fitToView();
  }

  setZoom(percent: number) {
    this.editor?.Canvas.setZoom(percent);
  }

  /** Zoom so the whole sheet (with a little desk around it) is visible and
   * centred — the natural view after a page-format change. */
  fitToView() {
    const editor = this.editor;
    if (!editor) {
      return;
    }
    // The frame wrapper takes its new size on the next layout pass.
    requestAnimationFrame(() => {
      if (this.editor !== editor) {
        return;
      }
      editor.Canvas.fitViewport({
        gap: 12,
        zoom: (zoom) => Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, Math.floor(zoom))),
      });
      this.zone.run(() => this.zoomChanged.emit(Math.round(editor.Canvas.getZoom())));
      // The frame wrapper animates to its new size (350 ms); redraw after.
      setTimeout(() => this.scheduleRulers(), 400);
    });
  }

  /** Palette click in visual mode: into the text being edited if any,
   * else as a new block at the page origin, selected. */
  insert(blockHtml: string, expression: string) {
    if (!this.editor || this.readonly) {
      return;
    }
    const editing = this.editor.getEditing();
    if (editing) {
      const doc = this.editor.Canvas.getDocument();
      doc?.execCommand('insertText', false, expression);
      return;
    }
    const added = this.editor.addComponents(blockHtml);
    const last = Array.isArray(added) ? added[added.length - 1] : added;
    if (last) {
      this.editor.select(last as GjsComponent);
    }
  }

  undo() {
    if (this.yamlPage) return;
    this.editor?.UndoManager.undo();
  }

  redo() {
    if (this.yamlPage) return;
    this.editor?.UndoManager.redo();
  }

  // --- setup ---

  private ensureCss() {
    if (document.querySelector(`link[href="${GRAPES_CSS}"]`)) {
      return;
    }
    this.cssLink = document.createElement('link');
    this.cssLink.rel = 'stylesheet';
    this.cssLink.href = GRAPES_CSS;
    document.head.appendChild(this.cssLink);
  }

  private init() {
    const self = this;
    const editor = grapesjs.init({
      container: this.canvasEl.nativeElement,
      height: '100%',
      width: 'auto',
      fromElement: false,
      storageManager: false,
      dragMode: 'absolute',
      panels: { defaults: [] },
      i18n: { locale: 'ru', messages: { ru } },
      canvas: {
        scripts: this.yamlPage ? [] : [`${environment.apiBaseUrl}/print_template/fit_text.js`],
      },
      traitManager: { appendTo: this.traitsEl.nativeElement },
      selectorManager: { componentFirst: true },
      // scrollLayers off: scrollIntoView on the selected layer would scroll
      // the whole side column and hide the traits/style panels above it.
      layerManager: { appendTo: this.layersEl.nativeElement, scrollLayers: false },
      styleManager: {
        appendTo: this.stylesEl.nativeElement,
        sectors: this.yamlPage ? yamlStyleSectors() : operatorStyleSectors(),
      },
    });
    this.editor = editor;
    if (this.yamlPage) {
      editor.UndoManager.stop();
      editor.Keymaps.remove('core:undo');
      editor.Keymaps.remove('core:redo');
    }

    this.registerTypes(editor);

    // The absolute dragger reads the start position with parseFloat(), so
    // `left: 14.5mm` would be taken as 14.5px and the block would jump on
    // mousedown. Teach it units instead.
    editor.Commands.extend('core:component-drag', {
      getPosition(this: { target: GjsComponent; isTran?: boolean; getTranslate(t: string, axis?: string): number }) {
        const style = this.target.getStyle() as Record<string, string>;
        if (this.isTran && style['transform']) {
          return { x: this.getTranslate(style['transform']), y: this.getTranslate(style['transform'], 'y') };
        }
        return { x: lengthToPx(style['left']), y: lengthToPx(style['top']) };
      },
    });

    editor.on('load', () => {
      this.applyPage();
      this.installCanvasChrome(editor);
      this.installRulers(editor);
      if (this.pending) {
        const pending = this.pending;
        this.pending = null;
        if (pending.projectData) {
          this.loadProjectData(pending.projectData, pending.html ?? '');
        } else if (pending.html !== undefined) {
          this.loadHtml(pending.html);
        }
      }
      this.zone.run(() => this.ready.emit());
    });

    // Any model change → the shell's dirty flag.
    editor.on('update', () => this.zone.run(() => this.changed.emit()));
    // Read-only: whatever gets in (paste, undo of nothing, a plugin) is
    // locked the same way as the loaded content.
    editor.on('component:add', (component: GjsComponent) => {
      if (this.readonly) this.lock(component);
    });
    editor.on('component:remove', (component: GjsComponent) => {
      if (this.loadingDoc || this.gesture || !this.yamlPage || this.readonly) return;
      const id = component.getAttributes()?.['data-yaml-id'];
      if (typeof id !== 'string' || !id) return;
      // A move detaches and reattaches in the same turn. A delete stays detached.
      queueMicrotask(() => {
        if (this.loadingDoc || this.gesture || !this.yamlPage || this.readonly) return;
        const root = this.editor?.getWrapper() as { find?: (query: string) => unknown[] } | undefined;
        if (root?.find?.(`[data-yaml-id="${id}"]`)?.length) return;
        this.zone.run(() => this.entryRemove.emit(id));
      });
    });

    // px → mm after a free drag / resize, snapped to the grid, so the
    // exported CSS reads `left: 12.5mm` and matches the print (раздел 6.2).
    editor.on('component:drag:start', () => (this.gesture = true));
    editor.on('component:drag:end', ({ target }) => {
      this.gesture = false;
      if (target) {
        this.toMm(target);
      }
    });
    // Resize is written in mm directly: GrapesJS's own update would keep
    // whatever unit the style has (unsnapped `51.858mm`) and rewrite
    // left/top as integer px measured from the frame, which drifts a
    // sheet-positioned element by up to half a grid step. Here width/height
    // come from the resizer's frame-px rect, left/top from the exact start
    // offsets plus the rect delta, all snapped to the grid.
    editor.on('component:resize:start', (props: ComponentResizeEventStartProps | ComponentResizeEventMoveProps) => {
      this.gesture = true;
      // GrapesJS 0.23 also fires this event on every pointer move (with the
      // move payload, no `model`); only the real start may reset the origin.
      if (!('model' in props)) {
        return;
      }
      const { rect, component, el } = props;
      const style = props.model.getStyle() as Record<string, string | undefined>;
      // A variable's text scales with the box height (like a design tool's
      // scale handle), so dragging a corner makes the label bigger, not
      // just its frame. Width-only handles leave the font alone: that is
      // how the wrap area is set. Images/codes have no font to scale.
      const scalesText = !this.yamlPage && ['tpl-var', 'tpl-text'].includes(String(component.get('type')));
      const fontPx = scalesText
        ? lengthToPx(style['font-size']) || parseFloat(el.ownerDocument.defaultView?.getComputedStyle(el).fontSize ?? '') || 0
        : 0;
      const codeKind = component.get('type') === 'tpl-code' ? (component.getAttributes()['data-code'] as CodeKind) : null;
      this.resizeStart = {
        leftMm: lengthToPx(style['left']) / PX_PER_MM,
        topMm: lengthToPx(style['top']) / PX_PER_MM,
        l: rect.l,
        t: rect.t,
        h: rect.h,
        fontPx,
        code: codeKind === 'barcode' || codeKind === 'qrcode' ? { component, kind: codeKind } : null,
      };
    });
    editor.on('component:resize:update', ({ rect, style, partial, updateStyle }: ComponentResizeEventUpdateProps) => {
      const start = this.resizeStart;
      if (!start) {
        return;
      }
      const snap = (mm: number) => Math.round(mm / GRID_MM) * GRID_MM;
      const next: Record<string, string> = {};
      if (start.code && !this.yamlPage) {
        // Codes must stay scannable (раздел 6.4): the width only takes
        // values with a whole number of printer dots per module, and the
        // `data-dots` trait follows. A barcode's height is free (taller
        // bars scan better, the SVG stretches); a QR stays square.
        const { kind, component } = start.code;
        const snapped = snapCodeWidthMm(kind, rect.w / PX_PER_MM, this.dpi, this.demo?.geometry.qr_modules ?? 29);
        next['width'] = `${snapped.widthMm}mm`;
        if (kind === 'qrcode') {
          next['height'] = `${snapped.widthMm}mm`;
        } else if ('height' in style) {
          next['height'] = `${snap(rect.h / PX_PER_MM)}mm`;
        }
        if (!partial && component.getAttributes()['data-dots'] !== String(snapped.dots)) {
          component.addAttributes({ 'data-dots': String(snapped.dots) });
        }
      } else {
        if ('width' in style) {
          next['width'] = `${snap(rect.w / PX_PER_MM)}mm`;
        }
        if ('height' in style) {
          next['height'] = `${snap(rect.h / PX_PER_MM)}mm`;
        }
      }
      if ('left' in style) {
        next['left'] = `${snap(start.leftMm + (rect.l - start.l) / PX_PER_MM)}mm`;
      }
      if ('top' in style) {
        next['top'] = `${snap(start.topMm + (rect.t - start.t) / PX_PER_MM)}mm`;
      }
      if ('height' in style && start.fontPx > 0 && start.h > 0) {
        const pt = (start.fontPx * (rect.h / start.h) * 72) / 96;
        next['font-size'] = `${Math.max(4, Math.round(pt * 2) / 2)}pt`;
      }
      updateStyle(next);
    });
    editor.on('component:resize:end', ({ component }: { component?: GjsComponent }) => {
      this.gesture = false;
      this.resizeStart = null;
      const target = component ?? editor.getSelected();
      if (target) {
        this.toMm(target);
      }
    });
    editor.on('component:styleUpdate', (component?: GjsComponent) => {
      if (!this.gesture && component && component !== editor.getWrapper()) {
        this.toMm(component);
      }
    });
    editor.on('component:update:attributes', (component: GjsComponent) => this.emitYamlBox(component, 'trait'));
    editor.on('component:selected', (component: GjsComponent) => {
      this.scheduleRulers();
      if (!this.yamlPage) return;
      if (component === editor.getWrapper()) {
        this.lastYamlId = null;
        return;
      }
      const id = component.getAttributes()?.['data-yaml-id'];
      if (typeof id === 'string' && id) {
        this.lastYamlId = id;
        if (!this.selectingBand) this.zone.run(() => this.entrySelect.emit(id));
      }
      const trait = component.getTrait?.('data-font');
      trait?.set('options', this.fontOptions());
    });
    editor.on('component:deselected', () => this.scheduleRulers());
    editor.on('component:hovered', (component: GjsComponent) => {
      this.hovered = component && component !== editor.getWrapper() ? component : null;
      this.scheduleRulers();
    });
    editor.on('component:unhovered', () => {
      this.hovered = null;
      this.scheduleRulers();
    });
    editor.on('component:drag component:resize', () => this.scheduleRulers());
    // Grapes may open edit without client coordinates. The frame `dblclick` path
    // supplies the caret point; this path must not pass {0,0} or it yanks the caret.
    editor.on('component:dblclick', (component: GjsComponent) => this.editPreentered(component));

    // Palette drop (HTML5 drag): GrapesJS appends the block at a px offset
    // it computes from screen coordinates, which is off once the canvas is
    // zoomed — so the block is re-placed under the pointer here. The drop
    // event itself arrives in frame coordinates on `canvas:dragend`.
    editor.on('canvas:drop', (dt: DataTransfer | null | undefined, model: GjsComponent | GjsComponent[]) => {
      if (this.yamlPage) {
        this.pendingYamlInsert = dt?.getData?.('application/x-yaml-insert') || null;
        const added = Array.isArray(model) ? model[0] : model;
        added?.remove();
        return;
      }
      this.dropped = Array.isArray(model) ? model[0] ?? null : model ?? null;
    });
    editor.on('canvas:dragend', (event?: Event) => {
      if (this.yamlPage) {
        const raw = this.pendingYamlInsert;
        this.pendingYamlInsert = null;
        const ev = event as DragEvent | undefined;
        const inFrame = ev?.target instanceof Node && ev.target.ownerDocument === editor.Canvas.getDocument();
        if (!raw || !ev || typeof ev.clientX !== 'number' || !inFrame) return;
        let insert: PaletteEntry;
        try {
          insert = JSON.parse(raw) as PaletteEntry;
        } catch {
          return;
        }
        const snap = (mm: number) => Math.round(mm / GRID_MM) * GRID_MM;
        insert.posX = snap(ev.clientX / PX_PER_MM - this.bodyOriginMm().x);
        insert.posY = snap(ev.clientY / PX_PER_MM - this.bodyOriginMm().y);
        this.zone.run(() => this.yamlDrop.emit(insert));
        return;
      }
      const dropped = this.dropped;
      this.dropped = null;
      const ev = event as DragEvent | undefined;
      if (!dropped || !ev || typeof ev.clientX !== 'number') {
        return;
      }
      const inFrame = ev.target instanceof Node && ev.target.ownerDocument === editor.Canvas.getDocument();
      if (!inFrame) {
        return;
      }
      const snap = (mm: number) => Math.max(0, Math.round(mm / GRID_MM) * GRID_MM);
      setTimeout(() => {
        this.gesture = false;
        dropped.addStyle({
          position: 'absolute',
          left: `${snap(ev.clientX / PX_PER_MM - this.bodyOriginMm().x)}mm`,
          top: `${snap(ev.clientY / PX_PER_MM - this.bodyOriginMm().y)}mm`,
        });
        this.toMm(dropped);
        editor.select(dropped);
      });
    });

    void self;
  }

  private toMm(component: GjsComponent) {
    const style = component.getStyle() as Record<string, string>;
    const next: Record<string, string> = {};
    let changed = false;
    for (const key of ['left', 'top', 'width', 'height']) {
      const value = style[key];
      if (typeof value === 'string' && /px$/.test(value)) {
        next[key] = `${pxToMm(parseFloat(value))}mm`;
        changed = true;
      }
    }
    if (changed) {
      component.addStyle(next);
    }
    this.emitYamlBox(component);
  }

  private emitYamlBox(component: GjsComponent | undefined, source: 'trait' | 'geometry' = 'geometry') {
    if (!this.yamlPage || this.loadingDoc || this.gesture || !component?.getAttributes) return;
    const attrs = component.getAttributes() as Record<string, string>;
    const id = attrs['data-yaml-id'];
    if (!id) return;
    const style = { ...(component.getStyle() as Record<string, string>) };
    this.zone.run(() => this.boxChange.emit({ id, style, attrs: { ...attrs }, source }));
  }

  private yamlTraitDefs(): Record<string, unknown>[] {
    return [
      {
        type: 'select',
        name: 'data-valign',
        label: 'По вертикали',
        options: [
          { id: 'top', name: 'сверху' },
          { id: 'middle', name: 'по центру' },
          { id: 'bottom', name: 'снизу' },
        ],
      },
      { type: 'checkbox', name: 'data-auto-scale', label: 'Уместить текст', valueTrue: 'true', valueFalse: 'false' },
      { type: 'number', name: 'data-leading', label: 'Интервал, mm', min: 0 },
      { type: 'text', name: 'data-if', label: 'Показывать если' },
      { type: 'select', name: 'data-font', label: 'Шрифт', options: this.fontOptions() },
    ];
  }

  private fontOptions(): { id: string; name: string }[] {
    const uploaded = this.fonts.filter((font) => font.id !== 'text');
    return [{ id: 'text', name: 'Liberation Serif' }, ...uploaded.map((font) => ({ id: font.id, name: font.label }))];
  }

  // --- rulers ---

  /** Two canvases over the desk showing mm from the sheet's top-left corner,
   * redrawn on zoom/pan/resize, with a pointer marker fed by the frame's
   * mousemove (frame coordinates are unscaled, so mm = px / 3.78 − desk).
   * While Select is active, the hovered box — or the selected one when
   * nothing is hovered — also marks its left/right and top/bottom edges. */
  private installRulers(editor: Editor) {
    const redraw = () => this.scheduleRulers();
    editor.on('canvas:update canvas:zoom canvas:coords frame:scroll', redraw);
    this.resizeObserver = new ResizeObserver(redraw);
    this.resizeObserver.observe(this.deskEl.nativeElement);
    const doc = editor.Canvas.getDocument();
    doc?.addEventListener('mousemove', (ev) => {
      const origin = this.bodyOriginMm();
      this.pointerMm = { x: ev.clientX / PX_PER_MM - origin.x, y: ev.clientY / PX_PER_MM - origin.y };
      this.scheduleRulers();
    });
    doc?.addEventListener('mouseleave', () => {
      this.pointerMm = null;
      this.scheduleRulers();
    });
    this.scheduleRulers();
  }

  private scheduleRulers() {
    cancelAnimationFrame(this.rulerRaf);
    this.rulerRaf = requestAnimationFrame(() => this.drawRulers());
  }

  private drawRulers() {
    const editor = this.editor;
    const frameEl = editor?.Canvas.getFrameEl();
    if (!editor || !frameEl) {
      return;
    }
    const desk = this.deskEl.nativeElement.getBoundingClientRect();
    const frame = frameEl.getBoundingClientRect();
    if (desk.width === 0 || frame.width === 0) {
      return;
    }
    const zoom = editor.Canvas.getZoom() / 100;
    const scale = PX_PER_MM * zoom; // screen px per mm
    const bodyOrigin = this.bodyOriginMm();
    const originX = frame.left - desk.left + bodyOrigin.x * scale;
    const originY = frame.top - desk.top + bodyOrigin.y * scale;
    const page = this.bodySize();
    const box = this.guideBox();
    this.drawRuler(this.rulerXEl.nativeElement, desk.width, RULER_PX, 'x', originX, scale, page.width, this.pointerMm?.x ?? null, box && [box.left, box.right]);
    this.drawRuler(this.rulerYEl.nativeElement, RULER_PX, desk.height - RULER_PX, 'y', originY - RULER_PX, scale, page.height, this.pointerMm?.y ?? null, box && [box.top, box.bottom]);
  }

  /** Edges of the box under the pointer, or of the selection when the
   * pointer is on the sheet itself. Millimetres from the sheet origin. */
  private guideBox(): { left: number; right: number; top: number; bottom: number } | null {
    if (this.tool !== 'select') return null;
    const editor = this.editor;
    if (!editor) return null;
    const component = this.hovered ?? editor.getSelected();
    if (!component || component === editor.getWrapper()) return null;
    return boxMm(component, this.bodyOriginMm());
  }

  private drawRuler(
    canvas: HTMLCanvasElement,
    cssW: number,
    cssH: number,
    axis: 'x' | 'y',
    origin: number,
    scale: number,
    pageMm: number,
    pointer: number | null,
    edges: [number, number] | null = null,
  ) {
    const dpr = window.devicePixelRatio || 1;
    const w = Math.max(1, Math.round(cssW));
    const h = Math.max(1, Math.round(cssH));
    if (canvas.width !== w * dpr || canvas.height !== h * dpr) {
      canvas.width = w * dpr;
      canvas.height = h * dpr;
    }
    const ctx = canvas.getContext('2d');
    if (!ctx) {
      return;
    }
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    const length = axis === 'x' ? w : h; // along the ruler
    const thick = axis === 'x' ? h : w;
    // Along-axis helpers so one routine draws both orientations.
    const line = (pos: number, from: number, to: number) => {
      const p = Math.round(pos) + 0.5;
      ctx.beginPath();
      if (axis === 'x') {
        ctx.moveTo(p, from);
        ctx.lineTo(p, to);
      } else {
        ctx.moveTo(from, p);
        ctx.lineTo(to, p);
      }
      ctx.stroke();
    };
    const band = (from: number, to: number, color: string) => {
      ctx.fillStyle = color;
      if (axis === 'x') ctx.fillRect(from, 0, to - from, thick);
      else ctx.fillRect(0, from, thick, to - from);
    };

    band(0, length, '#f3f4f6');
    band(origin, origin + pageMm * scale, '#ffffff');
    if (edges) {
      const from = origin + edges[0] * scale;
      const to = origin + edges[1] * scale;
      band(Math.min(from, to), Math.max(from, to), 'rgba(41, 109, 242, 0.18)');
    }
    // Corner square on the horizontal ruler covers the vertical one's top.
    if (axis === 'x') band(0, RULER_PX, '#e5e7eb');

    // Label step: the smallest "nice" step that leaves ≥ 44 px between labels.
    const steps = [1, 2, 5, 10, 20, 50, 100, 200, 500];
    const major = steps.find((s) => s * scale >= 44) ?? 500;
    const minor = major <= 2 ? major / 2 : major / 5;
    const fromMm = Math.floor((-origin / scale) / minor) * minor;
    const toMm = (length - origin) / scale;

    ctx.strokeStyle = '#9ca3af';
    ctx.lineWidth = 1;
    ctx.fillStyle = '#4b5563';
    ctx.font = '10px system-ui, sans-serif';
    ctx.textBaseline = 'top';
    for (let mm = fromMm; mm <= toMm; mm += minor) {
      const pos = origin + mm * scale;
      const isMajor = Math.abs(mm / major - Math.round(mm / major)) < 1e-6;
      const isMid = !isMajor && Math.abs((mm / major) * 2 - Math.round((mm / major) * 2)) < 1e-6;
      const tick = isMajor ? thick : isMid ? thick * 0.5 : thick * 0.3;
      line(pos, thick - tick, thick);
      if (isMajor) {
        const label = String(Math.round(mm));
        if (axis === 'x') {
          ctx.fillText(label, pos + 3, 2);
        } else {
          ctx.save();
          ctx.translate(2, pos - 3);
          ctx.rotate(-Math.PI / 2);
          ctx.fillText(label, 0, 0);
          ctx.restore();
        }
      }
    }
    // Sheet edges.
    ctx.strokeStyle = '#296df2';
    line(origin, 0, thick);
    line(origin + pageMm * scale, 0, thick);

    if (edges) {
      ctx.strokeStyle = '#296df2';
      ctx.font = '10px system-ui, sans-serif';
      ctx.textBaseline = 'top';
      const tips = edges.map((mm) => ({
        pos: origin + mm * scale,
        label: formatMm(mm),
      }));
      for (const tip of tips) line(tip.pos, 0, thick);
      // Start tip sits outside the span, end tip on the other side, so a
      // short box does not stack the two millimetre labels on one tick.
      // A filled badge keeps the number readable over the ruler's own labels.
      const [start, end] = tips[0].pos <= tips[1].pos ? [tips[0], tips[1]] : [tips[1], tips[0]];
      const badge = (label: string, x: number, y: number) => {
        const width = ctx.measureText(label).width;
        ctx.fillStyle = '#296df2';
        ctx.fillRect(x - 2, y - 1, width + 4, 12);
        ctx.fillStyle = '#ffffff';
        ctx.fillText(label, x, y);
      };
      if (axis === 'x') {
        const startWidth = ctx.measureText(start.label).width;
        badge(start.label, start.pos - startWidth - 4, 1);
        badge(end.label, end.pos + 3, 1);
      } else {
        const place = (tip: { pos: number; label: string }, above: boolean) => {
          ctx.save();
          ctx.translate(1, tip.pos);
          ctx.rotate(-Math.PI / 2);
          const width = ctx.measureText(tip.label).width;
          badge(tip.label, above ? 3 : -width - 3, 1);
          ctx.restore();
        };
        place(start, true);
        place(end, false);
      }
    }

    if (pointer !== null) {
      const pos = origin + pointer * scale;
      ctx.strokeStyle = '#ed293f';
      line(pos, 0, thick);
      ctx.fillStyle = '#ed293f';
      const label = `${(Math.round(pointer * 2) / 2).toFixed(1)}`;
      if (axis === 'x') {
        ctx.fillText(label, pos + 4, thick - 12);
      } else {
        ctx.save();
        ctx.translate(thick - 12, pos - 4);
        ctx.rotate(-Math.PI / 2);
        ctx.fillText(label, 0, 0);
        ctx.restore();
      }
    }
  }

  /** Page box on the wrapper (exported as the body rule — the template's
   * body *is* the sheet), plus canvas-only chrome: grey desk, dashed
   * margins, 5 mm grid. */
  private applyPage() {
    const editor = this.editor;
    if (!editor) {
      return;
    }
    const sheet = this.pageSize();
    const { width, height } = this.bodySize();
    // A plain CSS rule, not wrapper.setStyle(): the wrapper only accepts
    // background properties and silently drops the page box otherwise.
    // Exported by getCss() as `body { … }` — the template's body *is* the
    // sheet, so this is the right thing to persist too.
    editor.Css.setRule('body', {
      margin: '0',
      position: 'relative',
      width: `${width}mm`,
      height: `${height}mm`,
      overflow: this.yamlPage ? 'visible' : 'hidden',
      'background-color': '#ffffff',
    });
    // The frame itself is exactly sheet + desk, so GrapesJS can centre it
    // and zoom it as one object (otherwise the frame stretches to the
    // canvas and an A4 sheet just runs off the bottom of it).
    // A YAML page adds the bleed outside the trim. Boxes stay on the trim.
    const bleed = this.bleedMm();
    const frame = editor.Canvas.getFrame();
    if (frame) {
      frame.set({
        width: Math.round((sheet.width + bleed.left + bleed.right + 2 * DESK_PAD_MM) * PX_PER_MM),
        height: Math.round((sheet.height + bleed.top + bleed.bottom + 2 * DESK_PAD_MM) * PX_PER_MM),
      });
    }
    this.updateFrameStyle();
  }

  private lockIfReadonly() {
    const wrapper = this.editor?.getWrapper();
    if (this.readonly && wrapper) {
      this.lock(wrapper);
    }
  }

  private lock(component: GjsComponent) {
    component.set(LOCKED_PROPS);
    component.components().forEach((child: GjsComponent) => this.lock(child));
  }

  private margins(): { top: number; right: number; bottom: number; left: number } {
    const m = this.page.margins_mm;
    return { top: Number(m.top) || 0, right: Number(m.right) || 0, bottom: Number(m.bottom) || 0, left: Number(m.left) || 0 };
  }

  /** The template's body is the printable area: the printer lays it inside
   * the page margins (@page margin / padding on the raster's html), so a
   * body the size of the whole sheet would run off the right and bottom. */
  private bodySize(): { width: number; height: number } {
    const { width, height } = this.pageSize();
    if (this.yamlPage) return { width, height };
    const m = this.margins();
    return { width: Math.max(1, width - m.left - m.right), height: Math.max(1, height - m.top - m.bottom) };
  }

  /** Where the body's top-left corner sits in the frame, mm. */
  private bodyOriginMm(): { x: number; y: number } {
    const m = this.yamlPage ? { top: 0, left: 0 } : this.margins();
    const bleed = this.bleedMm();
    return { x: DESK_PAD_MM + bleed.left + m.left, y: DESK_PAD_MM + bleed.top + m.top };
  }

  private bleedMm(): { top: number; right: number; bottom: number; left: number } {
    if (!this.yamlPage || !this.bleeds) return { top: 0, right: 0, bottom: 0, left: 0 };
    return {
      top: Number(this.bleeds.top) || 0,
      right: Number(this.bleeds.right) || 0,
      bottom: Number(this.bleeds.bottom) || 0,
      left: Number(this.bleeds.left) || 0,
    };
  }

  private pageSize(): { width: number; height: number } {
    const standard: Record<string, [number, number]> = {
      A4: [210, 297], A5: [148, 210], A6: [105, 148], Letter: [215.9, 279.4], Legal: [215.9, 355.6],
    };
    let width = Number(this.page.width_mm ?? 100);
    let height = Number(this.page.height_mm ?? 100);
    if (this.page.format in standard) {
      [width, height] = standard[this.page.format];
      if (this.page.orientation === 'landscape') {
        [width, height] = [height, width];
      }
    }
    return { width, height };
  }

  private installCanvasChrome(editor: Editor) {
    this.updateFrameStyle();
    this.bindViewportGesture();
    this.bindDrawSession();
    this.bindPreenteredEdit();
    // GrapesJS rebuilds the frame document (project load, frame resize), and
    // the <style> in its head goes with it — without the desk padding the
    // body's white background spills over the whole frame.
    editor.on('canvas:frame:load', () => {
      this.updateFrameStyle();
      this.bindViewportGesture();
      this.bindDrawSession();
      this.bindPreenteredEdit();
    });
  }

  /** A double click types in the box itself. The frame document owns the listener. */
  private bindPreenteredEdit() {
    this.preenteredAbort?.abort();
    this.preenteredAbort = null;
    const doc = this.editor?.Canvas.getDocument();
    if (!doc) return;
    const abort = new AbortController();
    this.preenteredAbort = abort;
    doc.addEventListener(
      'dblclick',
      (event) => {
        const component = this.componentAt(event);
        if (!component || !this.editPreentered(component, { x: event.clientX, y: event.clientY })) return;
        event.preventDefault();
        event.stopPropagation();
      },
      { signal: abort.signal },
    );
  }

  private componentAt(event: Event): GjsComponent | null {
    return this.componentFrom(event.target);
  }

  private componentFrom(node: EventTarget | null): GjsComponent | null {
    if (!(node instanceof Element)) return null;
    const host = node.closest('[data-yaml-id]');
    if (!(host instanceof HTMLElement)) return null;
    const view = (host as HTMLElement & { __gjsv?: { model?: GjsComponent } }).__gjsv;
    return view?.model ?? null;
  }

  /**
   * Type in the box itself, in its own font. When `point` is set, the caret goes
   * there (frame dblclick). Omit it from `component:dblclick` so a second open
   * does not yank the caret to 0,0. Enter adds a line. Escape drops the edit.
   * Blur keeps it. The file is written only by Save.
   */
  private editPreentered(component: GjsComponent, point?: { x: number; y: number }): boolean {
    if (!this.yamlPage || this.readonly) return false;
    const type = String(component.get('type'));
    if (type !== 'tpl-var' && type !== 'tpl-text') return false;
    const attrs = component.getAttributes();
    const id = attrs['data-yaml-id'];
    const el = component.getEl?.();
    if (!id || !(el instanceof HTMLElement)) return false;
    this.beginTextEdit(id, point, el, attrs['data-preentered'] ?? '');
    return true;
  }

  /** Open the caret in that box. The text tool returns `edit-text` and the editor calls this. */
  beginTextEdit(id: string, client?: { x: number; y: number }, el?: HTMLElement, source?: string): void {
    const node =
      el ??
      (this.editor?.Canvas.getDocument()?.querySelector(`[data-yaml-id="${CSS.escape(id)}"]`) as HTMLElement | null);
    if (!(node instanceof HTMLElement)) return;
    this.textEdit.begin({
      el: node,
      id,
      source: source ?? node.getAttribute('data-preentered') ?? '',
      client,
    });
  }

  editingElement(): HTMLElement | null {
    return this.textEdit.editingElement();
  }

  placeGuard(): boolean {
    return this.textEdit.placeGuard();
  }

  /** Keep the typed text and close the caret. The shell calls this before leaving visual mode. */
  finishTextEdit(): void {
    this.textEdit.finish();
  }

  /** Select adds no listeners. Every other tool draws a band on this document. */
  private bindDrawSession() {
    this.drawSession?.dispose();
    this.drawSession = null;
    const doc = this.editor?.Canvas.getDocument();
    if (!doc) return;
    this.drawSession = new DrawPointerSession(doc, () => this.bodyOriginMm(), (rect, click, shift, target, point) => {
      this.zone.run(() => this.onDraw(rect, click, shift, target, point));
    });
    this.drawSession.setTool(this.drawMode());
    this.bindHistoryKey(doc);
  }

  private historyDoc: Document | null = null;
  private readonly onCanvasHistoryKey = (event: KeyboardEvent): void => {
    const editing = event.target as { isContentEditable?: boolean } | null;
    if (editing?.isContentEditable) return;
    this.zone.run(() => this.historyKey.emit(event));
    if (event.defaultPrevented) return;
    const key = event.key.toLowerCase();
    const mod = event.metaKey || event.ctrlKey;
    if (!this.yamlPage && mod && !event.altKey && key === 'z' && !event.shiftKey) {
      event.preventDefault();
      this.undo();
      return;
    }
    if (!this.yamlPage && mod && !event.altKey && ((key === 'z' && event.shiftKey) || (key === 'y' && !event.shiftKey))) {
      event.preventDefault();
      this.redo();
      return;
    }
    if (mod || event.altKey || (key !== 'backspace' && key !== 'delete')) return;
    const tag = (event.target as { tagName?: string } | null)?.tagName;
    if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return;
    event.preventDefault();
    event.stopPropagation();
    this.editor?.runCommand('core:component-delete');
  };

  /** Cmd+Z after a drag lands in the frame. The parent window never sees it. */
  private bindHistoryKey(doc: Document): void {
    if (this.historyDoc === doc) return;
    this.historyDoc?.removeEventListener('keydown', this.onCanvasHistoryKey, true);
    this.historyDoc = doc;
    doc.addEventListener('keydown', this.onCanvasHistoryKey, true);
  }

  /** Gesture and cursor come from the registry. Text yields the caret while typing. */
  private drawMode(): DrawMode {
    const tool = toolById(this.tool);
    return {
      gesture: tool.gesture,
      cursor: tool.cursor,
      onPointerDown: tool.onPointerDown ? (event) => tool.onPointerDown!(event, this.pointerHost()) : undefined,
    };
  }

  /** Narrow host for pointer-down. Append, style, and measure live on the editor ToolHost. */
  private pointerHost(): PointerHost {
    return {
      editingElement: () => this.textEdit.editingElement(),
    };
  }

  /** Hand the finished gesture to the editor. Entry is resolved there from the live page. */
  private onDraw(rect: DrawnRect | null, click: SheetPoint, shift = false, target: Element | null = null, point?: { x: number; y: number }): void {
    if (this.readonly) return;
    this.toolGesture.emit({
      click,
      rect,
      shift,
      client: point ?? { x: 0, y: 0 },
      target,
      entry: null,
    });
  }

  /** Two-finger pinch scales and moves the sheet together. A wheel without
   * Ctrl or ⌘ pans. Both go through GrapesJS zoom and canvas coordinates. */
  private bindViewportGesture() {
    this.viewportGesture?.destroy();
    this.viewportGesture = null;
    const doc = this.editor?.Canvas.getDocument();
    if (!doc) return;
    const gesture = createGesture([pinchAction, wheelAction]);
    this.viewportGesture = gesture(
      doc,
      {
        onPinch: (state) => this.onViewportPinch(state),
        onWheel: (state) => this.onViewportWheel(state),
      },
      {
        eventOptions: { passive: false, capture: true },
        pinch: {
          eventOptions: { passive: false, capture: true },
          preventDefault: true,
          triggerAllEvents: true,
          threshold: 0,
          from: () => [1, 0],
          pointer: { touch: true },
          modifierKey: ['ctrlKey', 'metaKey'],
        },
        wheel: {
          eventOptions: { passive: false, capture: true },
          preventDefault: true,
          triggerAllEvents: true,
        },
      },
    );
  }

  private onViewportPinch(state: FullGestureState<'pinch'>): { scale: number; pointer: { x: number; y: number } } | undefined {
    const canvas = this.editor?.Canvas;
    if (!canvas) return;
    state.event.preventDefault();
    state.event.stopPropagation();
    const pointer = this.canvasPointer(state.origin[0], state.origin[1]);
    if (!pointer) return;
    const scale = state.movement[0];
    // `movement[0]` is already the scale since this gesture started, so the
    // baseline is 1. Seeding it with the current movement would drop the first zoom.
    const memo = state.memo ?? { scale: 1, pointer };
    if (!(scale > 0)) return memo;
    // Current pose, not the pose from the start of the gesture: a two-finger
    // drag between zoom events has to stay under the midpoint.
    const next = viewportAfterPinch(
      { zoom: canvas.getZoom(), coords: canvas.getCoords(), origin: memo.pointer },
      scale / memo.scale,
      pointer,
      ZOOM_MIN,
      ZOOM_MAX,
    );
    canvas.setZoom(next.zoom);
    canvas.setCoords(next.coords.x, next.coords.y);
    this.zone.run(() => this.zoomChanged.emit(Math.round(next.zoom)));
    return { scale, pointer };
  }

  private onViewportWheel(state: FullGestureState<'wheel'>) {
    const event = state.event;
    // Ctrl or ⌘ marks the scale part of a pinch. The midpoint arrives as a
    // wheel without that modifier, including while the pinch is still active.
    if (event.ctrlKey || event.metaKey) return;
    const [dx, dy] = state.delta;
    if (dx === 0 && dy === 0) return;
    event.preventDefault();
    event.stopPropagation();
    const canvas = this.editor?.Canvas;
    if (!canvas) return;
    const next = viewportAfterPan(canvas.getCoords(), dx, dy);
    canvas.setCoords(next.x, next.y);
  }

  /** A point inside the frame, in the centered canvas space GrapesJS pans in. */
  private canvasPointer(clientX: number, clientY: number): { x: number; y: number } | null {
    const frame = this.editor?.Canvas.getFrameEl();
    const canvas = this.canvasEl?.nativeElement;
    if (!frame || !canvas) return null;
    const frameRect = frame.getBoundingClientRect();
    const canvasRect = canvas.getBoundingClientRect();
    const doc = frame.contentDocument?.documentElement;
    const scaleX = doc && doc.clientWidth > 0 ? frameRect.width / doc.clientWidth : 1;
    const scaleY = doc && doc.clientHeight > 0 ? frameRect.height / doc.clientHeight : 1;
    return {
      x: frameRect.left - canvasRect.left + clientX * scaleX - canvasRect.width / 2,
      y: frameRect.top - canvasRect.top + clientY * scaleY - canvasRect.height / 2,
    };
  }

  private updateFrameStyle() {
    const doc = this.editor?.Canvas.getDocument();
    if (!doc) {
      return;
    }
    if (!this.frameStyle || this.frameStyle.ownerDocument !== doc || !this.frameStyle.isConnected) {
      this.frameStyle = doc.createElement('style');
      this.frameStyle.setAttribute('data-canvas-chrome', '');
      doc.head.appendChild(this.frameStyle);
    }
    const m = this.yamlPage ? { top: 0, right: 0, bottom: 0, left: 0 } : this.margins();
    const bleed = this.bleedMm();
    const sheet = this.pageSize();
    const faces = this.yamlPage
      ? this.fontFaces.map((face) => `@font-face { font-family: '${face.family.replace(/'/g, '')}'; src: url('${face.url}'); }`).join('\n')
      : '';
    // Draw cursor lives only in DrawPointerSession (`style[data-draw-cursor]`).
    // The sheet (white, grid) is drawn on html::before; the body sits on it
    // inside the margins, exactly where the printer puts it. On a YAML page
    // the body is the trim and the padding outside it is the bleed.
    this.frameStyle.textContent = `
      ${faces}
      html { position: relative; background: ${DESK_COLOR}; box-sizing: border-box; overflow: hidden; touch-action: none;
             padding: ${DESK_PAD_MM + bleed.top + m.top}mm ${DESK_PAD_MM + bleed.right + m.right}mm ${DESK_PAD_MM + bleed.bottom + m.bottom}mm ${DESK_PAD_MM + bleed.left + m.left}mm; }
      html::before { content: ''; position: absolute; z-index: -1; pointer-events: none;
             left: ${DESK_PAD_MM + bleed.left}mm; top: ${DESK_PAD_MM + bleed.top}mm; width: ${sheet.width}mm; height: ${sheet.height}mm;
             background-color: #fff; box-shadow: 0 0 0 1px #cbd5e1, 0 4px 16px rgba(0,0,0,.08);
             background-image: linear-gradient(to right, rgba(0,0,0,.05) 1px, transparent 1px),
                               linear-gradient(to bottom, rgba(0,0,0,.05) 1px, transparent 1px);
             background-size: 5mm 5mm; }
      body[data-gjs-type="wrapper"] { min-height: 0 !important; padding-top: 0 !important; }
      body { outline: 1px dashed #94a3b8; outline-offset: 0;
             background-image: linear-gradient(to right, rgba(0,0,0,.05) 1px, transparent 1px),
                               linear-gradient(to bottom, rgba(0,0,0,.05) 1px, transparent 1px);
             background-size: 5mm 5mm; background-position: -${m.left}mm -${m.top}mm; }
      .tpl-var, .tpl-text, .tpl-code, .tpl-file { outline: 1px dotted rgba(41,109,242,.5); outline-offset: -1px; min-width: 4mm; min-height: 3mm; }
      .tpl-var, .tpl-text { white-space: pre-line; }
      [contenteditable='true'] { user-select: text !important; -webkit-user-select: text !important; cursor: text !important; }
    `;
  }

  // --- component types ---

  private registerTypes(editor: Editor) {
    const self = this;
    // GrapesJS re-applies *all* model attributes to the element on every
    // style/attribute update (`updateAttributes`), which puts the template
    // `src="{{ … }}"` back and blanks the demo picture set in onRender —
    // visible right after a resize. These wrappers restore it.
    type DemoView = { el: HTMLImageElement; model: GjsComponent; onRender(): void };
    const withDemoSrc = (baseView: { prototype: { updateAttributes(): void } }) => ({
      updateAttributes(this: DemoView) {
        baseView.prototype.updateAttributes.call(this);
        this.onRender();
      },
    });
    const DefaultView = editor.Components.getType('default')!.view as { prototype: { updateAttributes(): void } };
    const ImageView = editor.Components.getType('image')!.view as { prototype: { updateAttributes(): void } };
    const variableOptions = () =>
      self.groups.flatMap((g) => g.items.map((i) => ({ id: i.expr, name: `${i.label} (${g.title})`, list: i.isList })));

    // Free text block: an ordinary editable text (double click) that, like a
    // variable, is moved and resized by its box; `data-text` marks it.
    const textIsComponent = (el: Element) =>
      el instanceof HTMLElement && el.dataset['text'] !== undefined ? { type: 'tpl-text' } : undefined;
    if (this.yamlPage) {
      editor.Components.addType('tpl-text', {
        isComponent: textIsComponent,
        model: {
          defaults: {
            tagName: 'div',
            name: 'Текст',
            draggable: true,
            droppable: false,
            editable: false,
            resizable: true,
            attributes: { class: 'tpl-text' },
            traits: [
              { type: 'text', name: 'data-preentered', label: 'Текст' },
              ...this.yamlTraitDefs(),
            ],
          },
        },
      });
    } else {
      editor.Components.addType('tpl-text', {
        extend: 'text',
        isComponent: textIsComponent,
        model: {
          defaults: {
            name: 'Текст',
            draggable: true,
            resizable: true,
            attributes: { class: 'tpl-text' },
          },
        },
      });
    }

    editor.Components.addType('tpl-var', {
      isComponent: (el) => (el instanceof HTMLElement && el.dataset['var'] !== undefined ? { type: 'tpl-var' } : undefined),
      model: {
        defaults: {
          tagName: 'div',
          name: 'Переменная',
          draggable: true,
          droppable: false,
          editable: false,
          // Free resize (no ratio lock, unlike images): the box becomes the
          // area the value must fit into, see `data-fit` / `white-space`.
          resizable: true,
          attributes: { class: 'tpl-var' },
          traits: [
            { type: 'select', name: 'data-var', label: 'Переменная', options: [] },
            { type: 'text', name: 'data-prefix', label: 'Префикс' },
            { type: 'text', name: 'data-suffix', label: 'Суффикс' },
            {
              type: 'select', name: 'data-case', label: 'Регистр',
              options: [{ id: '', name: 'как есть' }, { id: 'upper', name: 'ВСЕ ЗАГЛАВНЫЕ' }, { id: 'title', name: 'С Заглавной' }],
            },
            { type: 'text', name: 'data-join', label: 'Разделитель (список)', placeholder: ', ' },
            { type: 'text', name: 'data-default', label: 'Если пусто' },
            ...(this.yamlPage
              ? this.yamlTraitDefs()
              : [
                  {
                    type: 'select',
                    name: 'data-fit',
                    label: 'Если не влезает',
                    options: [
                      { id: '', name: 'не менять' },
                      { id: 'min:10pt', name: 'уменьшать шрифт до 10pt' },
                      { id: 'min:8pt', name: 'уменьшать шрифт до 8pt' },
                      { id: 'min:6pt', name: 'уменьшать шрифт до 6pt' },
                    ],
                  },
                ]),
          ],
        },
        init(this: GjsComponent) {
          this.components().reset();
          const trait = this.getTrait('data-var');
          if (trait) {
            trait.set('options', variableOptions());
          }
          this.on('change:attributes', () => {
            const attrs = this.getAttributes();
            const option = variableOptions().find((o) => o.id === attrs['data-var']);
            if (option?.list) {
              this.addAttributes({ 'data-list': '1' });
            } else if (attrs['data-list']) {
              this.removeAttributes('data-list');
            }
            this.view?.render();
          });
        },
        toHTML(this: GjsComponent) {
          const attrs = this.getAttributes();
          const isList = attrs['data-list'] === '1';
          const expression = variableExpression(
            attrs['data-var'] ?? '',
            {
              case: (attrs['data-case'] as 'upper' | 'title' | '') || 'none',
              join: attrs['data-join'],
              default: attrs['data-default'],
            },
            isList
          );
          const attrString = Object.entries(this.getAttrToHTML())
            .filter(([, v]) => v !== undefined && v !== null && v !== '' && v !== false)
            .map(([k, v]) => (v === true ? k : `${k}="${escapeAttr(String(v))}"`))
            .join(' ');
          const text = `${attrs['data-prefix'] ?? ''}${expression}${attrs['data-suffix'] ?? ''}`;
          return `<div ${attrString}>${text}</div>`;
        },
      },
      view: {
        onRender(this: { el: HTMLElement; model: GjsComponent }) {
          const attrs = this.model.getAttributes();
          let value = demoValueFor(attrs['data-var'] ?? '', self.demo);
          if (attrs['data-case'] === 'upper') value = value.toUpperCase();
          if (attrs['data-case'] === 'title') value = value.replace(/\p{L}+/gu, (w) => w[0].toUpperCase() + w.slice(1).toLowerCase());
          if (!value && attrs['data-default']) value = attrs['data-default'];
          this.el.textContent = `${attrs['data-prefix'] ?? ''}${value}${attrs['data-suffix'] ?? ''}`;
        },
      },
    });

    const codeDotsOptions = (kind: Exclude<CodeKind, 'vcard'>) =>
      CODE_DOTS[kind].map((dots) => ({
        id: String(dots),
        name: `${dots} точ./модуль — ${Math.round(codeWidthMm(kind, dots, self.dpi, self.demo?.geometry.qr_modules ?? 29) * 100) / 100} мм`,
      }));

    editor.Components.addType('tpl-code', {
      isComponent: (el) => (el instanceof HTMLElement && el.dataset['code'] !== undefined ? { type: 'tpl-code' } : undefined),
      model: {
        defaults: {
          tagName: 'img',
          void: true,
          name: 'Код билета',
          draggable: true,
          droppable: false,
          // Overridden per kind in init(): a barcode resizes freely (its
          // height is independent of the module width), QR codes keep 1:1.
          resizable: { ratioDefault: true },
          attributes: { class: 'tpl-code' },
          traits: this.yamlPage
            ? []
            : [
                { type: 'select', name: 'data-variant', label: 'Формат', options: [{ id: 'svg', name: 'вектор (SVG)' }, { id: 'png', name: 'растр (PNG)' }] },
                { type: 'select', name: 'data-dots', label: 'Точек на модуль', options: [] },
              ],
        },
        init(this: GjsComponent) {
          const kind = (this.getAttributes()['data-code'] as CodeKind) || 'barcode';
          this.set('resizable', kind === 'barcode' ? true : { ratioDefault: true });
          const trait = this.getTrait('data-dots');
          if (trait) {
            trait.set('options', kind === 'vcard' ? [] : codeDotsOptions(kind));
          }
          this.on('change:attributes:data-dots', () => {
            const dots = Number(this.getAttributes()['data-dots']);
            if (dots && kind !== 'vcard') {
              this.addStyle({ width: `${codeWidthMm(kind, dots, self.dpi, self.demo?.geometry.qr_modules ?? 29)}mm` });
            }
          });
          this.on('change:attributes:data-variant', () => this.view?.render());
        },
        toHTML(this: GjsComponent) {
          const attrs = this.getAttributes();
          const kind = (attrs['data-code'] as CodeKind) || 'barcode';
          const variable = attrs['data-variant'] === 'png' ? CODE_VARIABLES[kind].png : CODE_VARIABLES[kind].svg;
          const attrString = Object.entries(this.getAttrToHTML())
            .filter(([k, v]) => k !== 'src' && v !== undefined && v !== null && v !== '' && v !== false)
            .map(([k, v]) => (v === true ? k : `${k}="${escapeAttr(String(v))}"`))
            .join(' ');
          return `<img ${attrString} src="{{ ${variable} }}">`;
        },
      },
      view: {
        ...withDemoSrc(DefaultView),
        onRender(this: { el: HTMLImageElement; model: GjsComponent }) {
          const attrs = this.model.getAttributes();
          const kind = (attrs['data-code'] as CodeKind) || 'barcode';
          const key = attrs['data-variant'] === 'png' ? CODE_VARIABLES[kind].png : CODE_VARIABLES[kind].svg;
          const src = self.demo?.codes[key as keyof DemoContext['codes']];
          if (src) {
            this.el.src = src;
          }
        },
      },
    });

    editor.Components.addType('tpl-file', {
      isComponent: (el) => (el instanceof HTMLElement && el.dataset['file'] !== undefined ? { type: 'tpl-file' } : undefined),
      model: {
        defaults: {
          tagName: 'img',
          void: true,
          name: 'Файл макета',
          draggable: true,
          droppable: false,
          resizable: { ratioDefault: true },
          attributes: { class: 'tpl-file' },
          traits: [{ type: 'select', name: 'data-file', label: 'Файл', options: [] }],
        },
        init(this: GjsComponent) {
          const trait = this.getTrait('data-file');
          if (trait) {
            trait.set('options', Object.keys(self.fileUrls).map((name) => ({ id: name, name })));
          }
          this.on('change:attributes:data-file', () => this.view?.render());
        },
        toHTML(this: GjsComponent) {
          const attrs = this.getAttributes();
          const attrString = Object.entries(this.getAttrToHTML())
            .filter(([k, v]) => k !== 'src' && v !== undefined && v !== null && v !== '' && v !== false)
            .map(([k, v]) => (v === true ? k : `${k}="${escapeAttr(String(v))}"`))
            .join(' ');
          return `<img ${attrString} src="{{ files.${attrs['data-file'] ?? ''} }}">`;
        },
      },
      view: {
        ...withDemoSrc(DefaultView),
        onRender(this: { el: HTMLImageElement; model: GjsComponent }) {
          const name = this.model.getAttributes()['data-file'] ?? '';
          const url = self.fileUrls[name];
          if (url) {
            this.el.src = url;
          }
        },
      },
    });

    // A plain <img src="{{ barcode }}"> / "{{ files.logo }}" (hand-written or
    // from the cloud template) stays a plain image in the export, but the
    // canvas shows the demo picture instead of a broken-image icon.
    // (No `extend:` here — re-adding an existing type keeps its isComponent;
    // with `extend` GrapesJS drops it and <img> stops being an image.)
    editor.Components.addType('image', {
      view: {
        ...withDemoSrc(ImageView),
        onRender(this: { el: HTMLImageElement; model: GjsComponent }) {
          const src = String(this.model.get('src') ?? this.model.getAttributes()['src'] ?? '');
          const url = self.demoSrcFor(src);
          if (url) {
            this.el.src = url;
          }
        },
      },
    });

    // Plain blocks for the palette's "Элементы" (text, line, box) come in as
    // ordinary GrapesJS text/default components — no custom type needed.
  }

  /** Demo URL for a `{{ variable }}` image source, or null if it isn't one. */
  private demoSrcFor(src: string): string | null {
    const match = /^\{\{\s*([\w.]+)\s*\}\}$/.exec(src.trim());
    if (!match) {
      return null;
    }
    const key = match[1];
    if (key.startsWith('files.')) {
      return this.fileUrls[key.slice('files.'.length)] ?? null;
    }
    return this.demo?.codes[key as keyof DemoContext['codes']] ?? null;
  }

  /** The shell calls this once demo data / files arrive after init, so the
   * canvas re-renders variable values and images. */
  refreshDemo() {
    const editor = this.editor;
    if (!editor) {
      return;
    }
    const visit = (component: GjsComponent) => {
      const type = component.get('type');
      // Components restored from saved state were built before the palette
      // and files arrived, so their select options are still empty.
      if (type === 'tpl-var') {
        component.getTrait('data-var')?.set(
          'options',
          this.groups.flatMap((g) => g.items.map((i) => ({ id: i.expr, name: `${i.label} (${g.title})`, list: i.isList })))
        );
      } else if (type === 'tpl-file') {
        component.getTrait('data-file')?.set('options', Object.keys(this.fileUrls).map((name) => ({ id: name, name })));
      }
      if (type === 'tpl-var' || type === 'tpl-code' || type === 'tpl-file' || type === 'image') {
        component.view?.render();
      }
      component.components().forEach(visit);
    };
    const wrapper = editor.getWrapper();
    if (wrapper) {
      visit(wrapper);
    }
  }
}
