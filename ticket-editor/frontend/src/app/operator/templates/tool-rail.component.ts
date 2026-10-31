import { Component, ElementRef, EventEmitter, HostListener, Input, NgZone, OnChanges, Output, ViewChild, inject, signal } from '@angular/core';
import { mount } from 'ticket-editor-color-picker';

import { normalizeHex, textStyleCache } from './text-style';
import { toolShortcuts, toolsInOrder, type EditorTool } from './tools/registry';
import type { SheetTool } from './tools/sheet-tool';

export type { EditorTool } from './tools/registry';
export type ColorTarget = 'foreground' | 'background';

export interface ColorPick {
  target: ColorTarget;
  hex: string;
}

type RailTool = SheetTool & { id: EditorTool };

/** Icon rail for the sheet. The active tool is owned by the editor; this strip shows it and emits the next one. */
@Component({
  selector: 'app-tool-rail',
  standalone: true,
  host: {
    class: 'relative flex w-[3.5rem] shrink-0 flex-col self-stretch',
    role: 'radiogroup',
    'aria-label': 'Tools',
  },
  styleUrl: './tool-rail.component.scss',
  template: `
    @for (tool of tools; track tool.id) {
      <button
        type="button"
        role="radio"
        [attr.aria-checked]="active() === tool.id"
        [attr.aria-pressed]="active() === tool.id"
        [attr.aria-label]="tool.label"
        [title]="tool.label"
        [disabled]="readonly && tool.creates"
        (click)="choose(tool.id)"
      >
        <i [class]="tool.icon" aria-hidden="true"></i>
      </button>
    }
    <div class="swatches">
      <button
        type="button"
        class="swap"
        aria-label="Swap colors"
        title="Swap colors (X)"
        (click)="swapColors()"
      >
        <svg viewBox="0 0 12 12" aria-hidden="true">
          <path d="M1 3.2h7.1M6.2 1.1 9.2 3.2 6.2 5.3" />
          <path d="M11 8.8H3.9M5.8 10.9 2.8 8.8 5.8 6.7" />
        </svg>
      </button>
      <button
        type="button"
        class="swatch background"
        aria-label="Background"
        title="Background"
        [style.background]="background"
        [attr.aria-pressed]="dialog() === 'background'"
        (click)="openSwatch('background')"
      ></button>
      <button
        type="button"
        class="swatch foreground"
        aria-label="Foreground"
        title="Foreground"
        [style.background]="foreground"
        [attr.aria-pressed]="dialog() === 'foreground'"
        (click)="openSwatch('foreground')"
      ></button>
    </div>
    @if (dialog(); as target) {
      <div class="color-backdrop" (mousedown)="dismiss()"></div>
      <div class="color-dialog" role="dialog" aria-label="Color" (keydown.escape)="dismiss($event)">
        <div role="tablist" aria-label="Picker">
          <button type="button" role="tab" [attr.aria-selected]="tab() === 'linear'" (click)="setTab('linear')">Linear</button>
          <button type="button" role="tab" [attr.aria-selected]="tab() === 'radial'" (click)="setTab('radial')">Radial</button>
        </div>
        <div #picker class="picker-host"></div>
        <div class="channels">
          <label>R <input aria-label="R" [value]="channel('r')" (input)="onChannel('r', $any($event.target).value)" /></label>
          <label>G <input aria-label="G" [value]="channel('g')" (input)="onChannel('g', $any($event.target).value)" /></label>
          <label>B <input aria-label="B" [value]="channel('b')" (input)="onChannel('b', $any($event.target).value)" /></label>
        </div>
        <label class="hex">Hex <input aria-label="Hex" [value]="hexText()" (input)="onHex($any($event.target).value)" /></label>
        <button type="button" class="apply" (click)="apply()">Apply</button>
      </div>
    }
  `,
})
export class ToolRailComponent implements OnChanges {
  @Input() tool: EditorTool = 'select';
  @Input() readonly = false;
  @Input() foreground = '#000000';
  @Input() background = '#FFFFFF';
  @Output() toolChange = new EventEmitter<EditorTool>();
  @Output() colorPick = new EventEmitter<ColorPick>();

  @ViewChild('picker') pickerHost?: ElementRef<HTMLElement>;

  readonly tools: RailTool[] = toolsInOrder();
  readonly active = signal<EditorTool>('select');
  readonly dialog = signal<ColorTarget | null>(null);
  readonly tab = signal<'linear' | 'radial'>('linear');
  readonly draft = signal('#000000');
  readonly hexText = signal('#000000');

  private readonly zone = inject(NgZone);
  private paintQueued = false;
  /** Color the open dialog started with. Dismiss writes it back; Apply keeps the draft. */
  private opened = '#000000';

  ngOnChanges(): void {
    this.active.set(this.tool);
  }

  choose(id: EditorTool): void {
    if (this.readonly && this.tools.find((tool) => tool.id === id)?.creates) return;
    this.active.set(id);
    this.toolChange.emit(id);
  }

  swapColors(): void {
    const foreground = normalizeHex(this.background) ?? '#000000';
    const background = normalizeHex(this.foreground) ?? '#FFFFFF';
    if (foreground === (normalizeHex(this.foreground) ?? '#000000') && background === (normalizeHex(this.background) ?? '#FFFFFF')) return;
    const style = textStyleCache.read();
    textStyleCache.remember({ ...style, foreground, background });
    this.colorPick.emit({ target: 'foreground', hex: foreground });
    this.colorPick.emit({ target: 'background', hex: background });
    const open = this.dialog();
    if (!open) return;
    const next = open === 'foreground' ? foreground : background;
    this.opened = next;
    this.setDraft(next);
    this.queuePaint();
  }

  openSwatch(target: ColorTarget): void {
    if (this.dialog() === target) {
      this.dismiss();
      return;
    }
    if (this.dialog()) this.dismiss();
    const color = normalizeHex(target === 'foreground' ? this.foreground : this.background) ?? '#000000';
    this.opened = color;
    this.dialog.set(target);
    this.tab.set('linear');
    this.setDraft(color);
    this.queuePaint();
  }

  /** Keep the draft and close. */
  apply(): void {
    const target = this.dialog();
    if (!target) return;
    const hex = normalizeHex(this.draft());
    if (!hex) return;
    this.write(target, hex);
    this.opened = hex;
    this.dialog.set(null);
  }

  /** Close and put back the color the dialog opened with. */
  dismiss(event?: Event): void {
    event?.stopPropagation();
    const target = this.dialog();
    if (!target) return;
    const opened = this.opened;
    this.dialog.set(null);
    if (textStyleCache.read()[target] !== opened) this.write(target, opened);
  }

  setTab(tab: 'linear' | 'radial'): void {
    this.tab.set(tab);
    this.queuePaint();
  }

  channel(part: 'r' | 'g' | 'b'): string {
    const hex = this.draft();
    const index = part === 'r' ? 1 : part === 'g' ? 3 : 5;
    return String(parseInt(hex.slice(index, index + 2), 16));
  }

  onChannel(part: 'r' | 'g' | 'b', raw: string): void {
    if (!/^\d{1,3}$/.test(raw.trim())) return;
    const value = Number(raw);
    if (!Number.isInteger(value) || value < 0 || value > 255) return;
    const hex = this.draft();
    const channels = {
      r: parseInt(hex.slice(1, 3), 16),
      g: parseInt(hex.slice(3, 5), 16),
      b: parseInt(hex.slice(5, 7), 16),
    };
    channels[part] = value;
    const next = `#${byte(channels.r)}${byte(channels.g)}${byte(channels.b)}`;
    this.preview(next, false);
  }

  onHex(raw: string): void {
    this.hexText.set(raw);
    const hex = normalizeHex(raw);
    if (!hex) return;
    this.preview(hex, false);
  }

  @HostListener('document:keydown', ['$event'])
  onKey(event: KeyboardEvent): void {
    const target = event.target;
    if (target instanceof HTMLElement && target.closest('input, textarea, select, [contenteditable="true"]')) return;
    // Sheet typing lives in the canvas iframe. Those keys do not bubble here as
    // contenteditable targets, but Playwright and some hosts still deliver them
    // to the parent document — do not steal tool shortcuts mid-edit.
    if (this.sheetIsEditing()) return;
    if (event.metaKey || event.ctrlKey || event.altKey) return;
    if (event.key === 'Escape') {
      if (this.dialog()) {
        this.dismiss();
        return;
      }
      this.choose('select');
      return;
    }
    if (event.key.toLowerCase() === 'x') {
      event.preventDefault();
      this.swapColors();
      return;
    }
    const next = toolShortcuts[event.key.toLowerCase()];
    if (!next) return;
    event.preventDefault();
    this.choose(next);
  }


  /** True when a box in the Grapes frame is contenteditable and focused. */
  private sheetIsEditing(): boolean {
    for (const frame of Array.from(document.querySelectorAll('iframe'))) {
      try {
        const active = frame.contentDocument?.activeElement;
        if (active instanceof HTMLElement && active.isContentEditable) return true;
      } catch {
        // Cross-origin frame.
      }
    }
    return false;
  }

  private setDraft(hex: string): void {
    this.draft.set(hex);
    this.hexText.set(hex);
  }

  private preview(hex: string, fromPicker: boolean): void {
    const target = this.dialog();
    if (!target) return;
    this.setDraft(hex);
    this.write(target, hex);
    if (!fromPicker) this.queuePaint();
  }

  private write(target: ColorTarget, hex: string): void {
    const style = textStyleCache.read();
    if (style[target] === hex) return;
    textStyleCache.remember({ ...style, [target]: hex });
    this.colorPick.emit({ target, hex });
  }

  private queuePaint(): void {
    if (this.paintQueued) return;
    this.paintQueued = true;
    setTimeout(() => {
      this.paintQueued = false;
      this.paintPicker();
    });
  }

  private paintPicker(): void {
    const host = this.pickerHost?.nativeElement;
    const target = this.dialog();
    if (!host || !target) return;
    mount(host, {
      color: this.draft(),
      mode: this.tab(),
      size: 180,
      onChange: (hex: string) => {
        const color = normalizeHex(hex);
        if (!color || color === this.draft()) return;
        this.zone.run(() => this.preview(color, true));
      },
    });
  }
}

function byte(value: number): string {
  return value.toString(16).padStart(2, '0').toUpperCase();
}
