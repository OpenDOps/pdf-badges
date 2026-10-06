import { useSyncExternalStore } from "react";

import { FormClient } from "../form/client";

export type DeskOption = { id: string; label: string };

export type DeskCatalog = {
  categories: DeskOption[];
  printers: DeskOption[];
  category: string;
  printer: string;
};

const EMPTY: DeskCatalog = { categories: [], printers: [], category: "", printer: "" };

export const DESK_CHOICE_KEY = "registration-form.desk";

let state: DeskCatalog = EMPTY;
let hasMemory = false;
let controller: AbortController | undefined;
const listeners = new Set<() => void>();

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function asList(body: unknown): unknown[] {
  if (Array.isArray(body)) return body;
  if (isRecord(body) && Array.isArray(body.data)) return body.data;
  if (isRecord(body) && Array.isArray(body.list)) return body.list;
  return [];
}

function textOf(value: unknown): string {
  if (typeof value === "string") return value;
  if (typeof value === "number") return String(value);
  if (!isRecord(value)) return "";
  const ru = value.ru;
  if (typeof ru === "string") return ru;
  if (isRecord(ru) && typeof ru.str === "string") return ru.str;
  return typeof value.str === "string" ? value.str : "";
}

function optionsOf(body: unknown, kind: "category" | "printer"): DeskOption[] {
  const options: DeskOption[] = [];
  for (const item of asList(body)) {
    if (typeof item === "string") {
      options.push({ id: item, label: item });
      continue;
    }
    if (!isRecord(item)) continue;
    if (kind === "category") {
      const id = item.cat_id ?? item.id;
      const label = textOf(item.name) || textOf(id);
      if (id === undefined || id === null || label === "") continue;
      options.push({ id: String(id), label });
      continue;
    }
    const id = textOf(item.value) || textOf(item.name) || textOf(item.id);
    const label = textOf(item.text) || textOf(item.name) || id;
    if (!id) continue;
    options.push({ id, label });
  }
  return options;
}

function emit() {
  listeners.forEach((listener) => listener());
}

function kept(options: DeskOption[], id: string): string {
  return options.some((item) => item.id === id) ? id : "";
}

function keptCategory(options: DeskOption[], id: string): string {
  if (options.some((item) => item.id === id)) return id;
  return options[0]?.id ?? "";
}

function readStored(): { category: string; printer: string } | undefined {
  try {
    const raw = localStorage.getItem(DESK_CHOICE_KEY);
    if (!raw) return undefined;
    const parsed: unknown = JSON.parse(raw);
    if (!isRecord(parsed)) return undefined;
    if (typeof parsed.category !== "string" && typeof parsed.printer !== "string") return undefined;
    return {
      category: typeof parsed.category === "string" ? parsed.category : "",
      printer: typeof parsed.printer === "string" ? parsed.printer : "",
    };
  } catch {
    return undefined;
  }
}

function writeStored(choice: { category: string; printer: string }) {
  try {
    localStorage.setItem(DESK_CHOICE_KEY, JSON.stringify({ category: choice.category, printer: choice.printer }));
  } catch {
    // A private window or a full store still keeps the in-memory choice.
  }
}

function remembered(): { category: string; printer: string } {
  if (hasMemory) return { category: state.category, printer: state.printer };
  hasMemory = true;
  const stored = readStored();
  if (!stored) return { category: state.category, printer: state.printer };
  return stored;
}

function publish(body: { categories: unknown; printers: unknown }) {
  const categories = optionsOf(body.categories, "category");
  const printers = optionsOf(body.printers, "printer");
  const chosen = remembered();
  const next: DeskCatalog = {
    categories,
    printers,
    category: keptCategory(categories, chosen.category),
    printer: kept(printers, chosen.printer),
  };
  if (
    next.category === state.category &&
    next.printer === state.printer &&
    next.categories.length === state.categories.length &&
    next.printers.length === state.printers.length &&
    next.categories.every((item, index) => item.id === state.categories[index]?.id && item.label === state.categories[index]?.label) &&
    next.printers.every((item, index) => item.id === state.printers[index]?.id && item.label === state.printers[index]?.label)
  ) {
    return;
  }
  state = next;
  writeStored({ category: next.category, printer: next.printer });
  emit();
}

function isAbort(error: unknown): boolean {
  return error instanceof DOMException && error.name === "AbortError";
}

function delay(ms: number, signal: AbortSignal): Promise<void> {
  if (signal.aborted) return Promise.reject(new DOMException("aborted", "AbortError"));
  return new Promise((resolve, reject) => {
    const timer = setTimeout(resolve, ms);
    signal.addEventListener(
      "abort",
      () => {
        clearTimeout(timer);
        reject(new DOMException("aborted", "AbortError"));
      },
      { once: true },
    );
  });
}

async function poll(signal: AbortSignal) {
  const client = new FormClient();
  let rev = 0;
  let seen = false;
  while (!signal.aborted) {
    try {
      const data = await client.catalog(rev, signal);
      if (signal.aborted) return;
      if (!seen || data.rev !== rev) {
        seen = true;
        rev = data.rev;
        publish(data);
        continue;
      }
      await delay(250, signal);
    } catch (error) {
      if (signal.aborted || isAbort(error)) return;
      try {
        await delay(1000, signal);
      } catch {
        return;
      }
    }
  }
}

export function subscribeDeskCatalog(listener: () => void) {
  listeners.add(listener);
  if (!controller) {
    controller = new AbortController();
    void poll(controller.signal);
  }
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0 && controller) {
      controller.abort();
      controller = undefined;
    }
  };
}

export function deskCatalogSnapshot(): DeskCatalog {
  return state;
}

function serverCatalog(): DeskCatalog {
  return EMPTY;
}

export function useDeskCatalog(): DeskCatalog {
  return useSyncExternalStore(subscribeDeskCatalog, deskCatalogSnapshot, serverCatalog);
}

export function chooseDeskCatalog(patch: { category?: string; printer?: string }) {
  hasMemory = true;
  const next = { ...state };
  if (patch.category !== undefined) {
    next.category = patch.category === "" ? state.categories[0]?.id ?? "" : patch.category;
  }
  if (patch.printer !== undefined) next.printer = patch.printer;
  state = next;
  writeStored({ category: next.category, printer: next.printer });
  emit();
}

export function resetDeskCatalog() {
  controller?.abort();
  controller = undefined;
  listeners.clear();
  hasMemory = false;
  state = EMPTY;
}
