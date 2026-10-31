import {
  AfterViewInit,
  Component,
  ElementRef,
  EventEmitter,
  Input,
  OnDestroy,
  Output,
  ViewChild,
} from '@angular/core';
import { autocompletion, CompletionContext, CompletionResult } from '@codemirror/autocomplete';
import { css } from '@codemirror/lang-css';
import { html } from '@codemirror/lang-html';
import { json } from '@codemirror/lang-json';
import { Compartment, EditorState, StateEffect, StateField } from '@codemirror/state';
import { Decoration, DecorationSet, EditorView, ViewPlugin, ViewUpdate, keymap } from '@codemirror/view';
import { indentWithTab } from '@codemirror/commands';
import { basicSetup } from 'codemirror';

export interface CompletionItem {
  label: string;
  detail?: string;
  /** What gets inserted — defaults to label. */
  apply?: string;
}

/** CodeMirror 6 wrapper for the template's code mode (docs/
 * TZ_print_templates.md, раздел 6.5): HTML (or CSS) with `{{ }}` / `{% %}`
 * highlighted on top, variable autocompletion after `{{ `, a line marker
 * for server-side validation errors, and insert-at-cursor for the palette. */
@Component({
  selector: 'app-code-editor',
  standalone: true,
  template: `<div #host class="h-full min-h-[320px] border border-border-grey-light rounded-lg overflow-hidden text-sm"></div>`,
})
export class CodeEditorComponent implements AfterViewInit, OnDestroy {
  @ViewChild('host', { static: true }) host!: ElementRef<HTMLDivElement>;

  @Input() language: 'html' | 'css' | 'json' = 'html';
  @Input() readonly = false;
  @Input() set value(value: string) {
    this.pendingValue = value ?? '';
    if (this.view && this.view.state.doc.toString() !== this.pendingValue) {
      this.view.dispatch({ changes: { from: 0, to: this.view.state.doc.length, insert: this.pendingValue } });
    }
  }
  @Input() set completions(items: CompletionItem[]) {
    this.completionItems = items ?? [];
  }
  @Input() set errorLine(line: number | null) {
    this.pendingErrorLine = line;
    this.applyErrorLine();
  }

  @Output() valueChange = new EventEmitter<string>();

  private view: EditorView | null = null;
  private pendingValue = '';
  private pendingErrorLine: number | null = null;
  private completionItems: CompletionItem[] = [];
  private readonly readonlyCompartment = new Compartment();

  ngAfterViewInit(): void {
    const setErrorLine = StateEffect.define<number | null>();
    const errorLineField = StateField.define<DecorationSet>({
      create: () => Decoration.none,
      update: (deco, tr) => {
        deco = deco.map(tr.changes);
        for (const effect of tr.effects) {
          if (effect.is(setErrorLine)) {
            if (effect.value === null || effect.value < 1 || effect.value > tr.state.doc.lines) {
              deco = Decoration.none;
            } else {
              const line = tr.state.doc.line(effect.value);
              deco = Decoration.set([Decoration.line({ class: 'cm-error-line' }).range(line.from)]);
            }
          }
        }
        return deco;
      },
      provide: (f) => EditorView.decorations.from(f),
    });
    this.setErrorLineEffect = setErrorLine;

    const templateTagHighlighter = ViewPlugin.fromClass(
      class {
        decorations: DecorationSet;
        constructor(view: EditorView) {
          this.decorations = this.build(view);
        }
        update(update: ViewUpdate) {
          if (update.docChanged || update.viewportChanged) {
            this.decorations = this.build(update.view);
          }
        }
        build(view: EditorView): DecorationSet {
          const marks: ReturnType<typeof Decoration.mark>[] = [];
          const ranges: { from: number; to: number; cls: string }[] = [];
          for (const { from, to } of view.visibleRanges) {
            const text = view.state.doc.sliceString(from, to);
            const re = /\{\{[\s\S]*?\}\}|\{%[\s\S]*?%\}/g;
            let match: RegExpExecArray | null;
            while ((match = re.exec(text))) {
              ranges.push({
                from: from + match.index,
                to: from + match.index + match[0].length,
                cls: match[0].startsWith('{{') ? 'cm-tpl-var' : 'cm-tpl-tag',
              });
            }
          }
          void marks;
          return Decoration.set(
            ranges.map((r) => Decoration.mark({ class: r.cls }).range(r.from, r.to)),
            true
          );
        }
      },
      { decorations: (v) => v.decorations }
    );

    const complete = (context: CompletionContext): CompletionResult | null => {
      const before = context.matchBefore(/\{\{\s*[\w.]*/);
      if (!before && !context.explicit) {
        return null;
      }
      const from = before ? before.from + before.text.indexOf('{{') + 2 : context.pos;
      const typed = before ? before.text.slice(before.text.indexOf('{{') + 2) : '';
      const lead = typed.match(/^\s*/)?.[0] ?? '';
      return {
        from: from + lead.length,
        options: this.completionItems.map((item) => ({
          label: item.label,
          detail: item.detail,
          apply: item.apply ?? item.label,
          type: 'variable',
        })),
        validFor: /^[\w.]*$/,
      };
    };

    const theme = EditorView.theme({
      '&': { height: '100%', fontSize: '13px' },
      '.cm-scroller': { fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace' },
      '.cm-tpl-var': { color: '#296df2', fontWeight: '600' },
      '.cm-tpl-tag': { color: '#b45309', fontWeight: '600' },
      '.cm-error-line': { backgroundColor: '#fdeaec' },
      '.cm-foldGutter span': { color: '#334155', fontSize: '14px', padding: '0 3px' },
      '.cm-foldPlaceholder': {
        backgroundColor: '#e8eefc',
        border: '1px solid #c5d4f5',
        color: '#1e3a8a',
        borderRadius: '4px',
        margin: '0 4px',
        padding: '0 6px',
      },
    });

    this.view = new EditorView({
      parent: this.host.nativeElement,
      state: EditorState.create({
        doc: this.pendingValue,
        extensions: [
          basicSetup,
          keymap.of([indentWithTab]),
          ...this.languageSupport(),
          templateTagHighlighter,
          errorLineField,
          autocompletion({ override: [complete], activateOnTyping: true }),
          theme,
          this.readonlyCompartment.of(EditorState.readOnly.of(this.readonly)),
          EditorView.updateListener.of((update) => {
            if (update.docChanged) {
              this.valueChange.emit(update.state.doc.toString());
            }
          }),
        ],
      }),
    });
    this.applyErrorLine();
  }

  private languageSupport() {
    if (this.language === 'css') return [css()];
    if (this.language === 'json') return [json()];
    return [html({ matchClosingTags: true, autoCloseTags: true })];
  }

  private setErrorLineEffect: ReturnType<typeof StateEffect.define<number | null>> | null = null;

  ngOnDestroy(): void {
    this.view?.destroy();
    this.view = null;
  }

  insertAtCursor(text: string) {
    if (!this.view) {
      return;
    }
    const { from, to } = this.view.state.selection.main;
    this.view.dispatch({ changes: { from, to, insert: text }, selection: { anchor: from + text.length } });
    this.view.focus();
  }

  setReadonly(readonly: boolean) {
    this.readonly = readonly;
    this.view?.dispatch({ effects: this.readonlyCompartment.reconfigure(EditorState.readOnly.of(readonly)) });
  }

  private applyErrorLine() {
    if (!this.view || !this.setErrorLineEffect) {
      return;
    }
    this.view.dispatch({ effects: this.setErrorLineEffect.of(this.pendingErrorLine) });
    if (this.pendingErrorLine && this.pendingErrorLine <= this.view.state.doc.lines) {
      const line = this.view.state.doc.line(this.pendingErrorLine);
      this.view.dispatch({ effects: EditorView.scrollIntoView(line.from, { y: 'center' }) });
    }
  }
}
