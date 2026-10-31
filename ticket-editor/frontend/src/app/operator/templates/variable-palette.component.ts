import { Component, EventEmitter, Input, Output, computed, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';

import { DemoContext } from '../../core/models/print.models';
import { flatMustache } from './page-view';
import {
  CodeKind,
  PaletteGroup,
  PaletteItem,
  codeBlockHtml,
  codeWidthMm,
  fileImageHtml,
  variableBlockHtml,
  variableExpression,
} from './template-snippets';

export interface PaletteInsert {
  /** Inline expression for the code editor / text caret: `{{ … }}` or an <img>. */
  expression: string;
  /** Standalone block for the visual canvas. */
  blockHtml: string;
  /** Set on `/editor/:id`, where the host appends a page entry instead of HTML. */
  pageKind?: 'var' | 'text' | 'code' | 'file';
  editorPath?: string;
  codeKind?: 'barcode' | 'qrcode';
  fileName?: string;
}

interface CodeItem {
  kind: CodeKind;
  label: string;
  hint: string;
}

/** Left panel of the editor (docs/TZ_print_templates.md, разделы 6.3–6.4):
 * variables from the survey, ticket-code blocks sized in printer dots, and
 * the template's files. Click → `insert` (the host decides what to do with
 * it per mode); drag → native HTML5 drag with the block as `text/html`,
 * which the GrapesJS canvas accepts as a drop. */
@Component({
  selector: 'app-variable-palette',
  standalone: true,
  imports: [FormsModule],
  template: `
    <div class="flex flex-col gap-3 text-sm">
      <input
        [(ngModel)]="query"
        placeholder="Поиск переменной…"
        class="px-3 py-1.5 rounded-lg border border-border-grey-medium"
      />

      @for (group of filteredGroups(); track group.title) {
        <div>
          <p class="text-text-secondary text-xs uppercase tracking-wide mb-1">{{ group.title }}</p>
          <ul class="flex flex-col gap-1">
            @for (item of group.items; track item.expr) {
              <li
                class="px-2 py-1 rounded-lg bg-bg-grey hover:bg-button-base cursor-grab"
                draggable="true"
                [title]="'{{ ' + item.expr + ' }}'"
                (dragstart)="dragVariable($event, item)"
                (click)="clickVariable(item)"
              >
                <span class="text-text-base">{{ item.label }}</span>
                @if (item.hint) {
                  <span class="text-text-secondary"> · {{ item.hint }}</span>
                }
                <span class="block text-xs text-text-secondary truncate">{{ item.sample }}</span>
              </li>
            }
          </ul>
        </div>
      }

      @if (!query) {
        <div>
          <p class="text-text-secondary text-xs uppercase tracking-wide mb-1">Элементы</p>
          <ul class="flex flex-col gap-1">
            <li
              class="px-2 py-1 rounded-lg bg-bg-grey hover:bg-button-base cursor-grab"
              draggable="true"
              (dragstart)="dragText($event)"
              (click)="clickText()"
            >
              <span class="text-text-base">Текстовый блок</span>
              <span class="block text-xs text-text-secondary">произвольный текст, двойной клик — править</span>
            </li>
          </ul>
        </div>

        <div>
          <p class="text-text-secondary text-xs uppercase tracking-wide mb-1">Коды билета</p>
          <ul class="flex flex-col gap-1">
            @for (item of codeItems; track item.kind) {
              <li
                class="px-2 py-1 rounded-lg bg-bg-grey hover:bg-button-base"
                [class.opacity-50]="item.kind === 'vcard' && !demo?.has_vcard"
                [class.cursor-grab]="item.kind !== 'vcard' || demo?.has_vcard"
                [draggable]="item.kind !== 'vcard' || !!demo?.has_vcard"
                (dragstart)="dragCode($event, item.kind)"
                (click)="clickCode(item.kind)"
                [title]="item.kind === 'vcard' && !demo?.has_vcard ? 'У вида билета нет шаблона vCard' : ''"
              >
                <span class="text-text-base">{{ item.label }}</span>
                <span class="block text-xs text-text-secondary">{{ item.hint }}</span>
              </li>
            }
            <li class="px-2 py-1 text-xs text-text-secondary">
              Размер кодов задаётся в точках принтера на модуль ({{ dpi }} dpi):
              EAN-13 {{ widthLabel('barcode', 2) }} / {{ widthLabel('barcode', 3) }} / {{ widthLabel('barcode', 4) }} / {{ widthLabel('barcode', 5) }} мм,
              QR {{ widthLabel('qrcode', 3) }} / {{ widthLabel('qrcode', 4) }} / {{ widthLabel('qrcode', 5) }} мм.
            </li>
          </ul>
        </div>

        <div>
          <p class="text-text-secondary text-xs uppercase tracking-wide mb-1">Файлы макета</p>
          @if (fileNames().length === 0) {
            <p class="text-xs text-text-secondary px-2">Нет файлов — загрузите логотип или фон ниже.</p>
          } @else {
            <ul class="flex flex-col gap-1">
              @for (name of fileNames(); track name) {
                <li
                  class="px-2 py-1 rounded-lg bg-bg-grey hover:bg-button-base cursor-grab"
                  draggable="true"
                  [title]="'{{ files.' + name + ' }}'"
                  (dragstart)="dragFile($event, name)"
                  (click)="clickFile(name)"
                >
                  <span class="text-text-base">{{ name }}</span>
                  <span class="block text-xs text-text-secondary">{{ files[name] }}</span>
                </li>
              }
            </ul>
          }
        </div>
      }
    </div>
  `,
})
export class VariablePaletteComponent {
  @Input() set groups(value: PaletteGroup[]) {
    this.allGroups.set(value ?? []);
  }
  @Input() demo: DemoContext | null = null;
  @Input() set manifest(value: Record<string, string>) {
    this.files = value ?? {};
    this.fileNames.set(Object.keys(this.files));
  }
  @Input() dpi = 203;
  /** When set, a click carries a page entry instead of a Django block. */
  @Input() yamlPage = false;

  @Output() insert = new EventEmitter<PaletteInsert>();

  query = '';
  files: Record<string, string> = {};
  readonly fileNames = signal<string[]>([]);
  private readonly allGroups = signal<PaletteGroup[]>([]);

  readonly codeItems: CodeItem[] = [
    { kind: 'barcode', label: 'Штрихкод EAN-13', hint: 'код билета, вектор' },
    { kind: 'qrcode', label: 'QR-код билета', hint: 'код билета, вектор' },
    { kind: 'vcard', label: 'QR vCard', hint: 'контакт гостя из анкеты' },
  ];

  readonly filteredGroups = computed(() => {
    const q = this.query.trim().toLowerCase();
    const groups = this.allGroups();
    if (!q) {
      return groups;
    }
    return groups
      .map((g) => ({ ...g, items: g.items.filter((i) => i.label.toLowerCase().includes(q) || i.expr.includes(q)) }))
      .filter((g) => g.items.length > 0);
  });

  private get qrModules(): number {
    return this.demo?.geometry.qr_modules ?? 29;
  }

  widthLabel(kind: CodeKind, dots: number): string {
    return String(Math.round(codeWidthMm(kind, dots, this.dpi, this.qrModules) * 100) / 100);
  }

  private variableInsert(item: PaletteItem): PaletteInsert {
    const expression = this.yamlPage ? flatMustache(item.expr) : variableExpression(item.expr, {}, item.isList);
    return {
      expression,
      blockHtml: variableBlockHtml(item, {}, 'position:absolute;left:5mm;top:5mm;font-size:12pt;white-space:nowrap'),
      pageKind: this.yamlPage ? 'var' : undefined,
      editorPath: this.yamlPage ? item.expr : undefined,
    };
  }

  private codeInsert(kind: CodeKind): PaletteInsert {
    const dots = kind === 'barcode' ? 3 : 4;
    const widthMm = codeWidthMm(kind, dots, this.dpi, this.qrModules);
    const block = codeBlockHtml(kind, { widthMm, heightMm: 15 });
    return {
      expression: block,
      blockHtml: block,
      pageKind: this.yamlPage && kind !== 'vcard' ? 'code' : undefined,
      codeKind: kind === 'barcode' || kind === 'qrcode' ? kind : undefined,
    };
  }

  private textInsert(): PaletteInsert {
    const text = 'Текст';
    return {
      expression: text,
      blockHtml: `<div class="tpl-text" data-text="1" style="position:absolute;left:5mm;top:5mm;font-size:12pt">${text}</div>`,
      pageKind: this.yamlPage ? 'text' : undefined,
    };
  }

  private fileInsert(name: string): PaletteInsert {
    return {
      expression: `{{ files.${name} }}`,
      blockHtml: fileImageHtml(name),
      pageKind: this.yamlPage ? 'file' : undefined,
      fileName: this.yamlPage ? name : undefined,
    };
  }

  clickVariable(item: PaletteItem) {
    this.insert.emit(this.variableInsert(item));
  }

  clickCode(kind: CodeKind) {
    if (kind === 'vcard' && !this.demo?.has_vcard) {
      return;
    }
    this.insert.emit(this.codeInsert(kind));
  }

  clickText() {
    this.insert.emit(this.textInsert());
  }

  dragText(event: DragEvent) {
    this.setDragData(event, this.textInsert());
  }

  clickFile(name: string) {
    this.insert.emit(this.fileInsert(name));
  }

  dragVariable(event: DragEvent, item: PaletteItem) {
    this.setDragData(event, this.variableInsert(item));
  }

  dragCode(event: DragEvent, kind: CodeKind) {
    if (kind === 'vcard' && !this.demo?.has_vcard) {
      event.preventDefault();
      return;
    }
    this.setDragData(event, this.codeInsert(kind));
  }

  dragFile(event: DragEvent, name: string) {
    this.setDragData(event, this.fileInsert(name));
  }

  private setDragData(event: DragEvent, insert: PaletteInsert) {
    event.dataTransfer?.setData('text/html', insert.blockHtml);
    event.dataTransfer?.setData('text/plain', insert.expression);
    if (insert.pageKind) {
      event.dataTransfer?.setData(
        'application/x-yaml-insert',
        JSON.stringify({
          pageKind: insert.pageKind,
          editorPath: insert.editorPath,
          codeKind: insert.codeKind,
          fileName: insert.fileName,
        }),
      );
    }
    if (event.dataTransfer) {
      event.dataTransfer.effectAllowed = 'copy';
    }
  }
}
