import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";

import { FormClient } from "../form/client";
import { ClosedSelect } from "../picker";
import { useDeskCatalog } from "../shell/catalog";
import { ScreenFrame } from "../shell/header";

export const PRINTER_POLL_MS = 2000;

export type PrinterParam = {
  name: string;
  value: string;
  values: string[];
};

export type PrinterRow = {
  id: string;
  name: string;
  hidden: boolean;
  default: boolean;
  available: boolean;
  zone: string;
  category: string;
  print_cert: boolean;
  kind: string;
  params: PrinterParam[];
};

type Draft = PrinterRow & { saved: boolean };

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function zoneDigits(value: string): string {
  return /^\d+$/.test(value) ? value : "";
}

function printerParams(value: unknown): PrinterParam[] {
  if (!Array.isArray(value)) return [];
  const params: PrinterParam[] = [];
  for (const item of value) {
    if (!isRecord(item) || typeof item.name !== "string" || item.name === "") continue;
    const values = Array.isArray(item.values)
      ? item.values.filter((entry): entry is string => typeof entry === "string")
      : [];
    params.push({
      name: item.name,
      value: typeof item.value === "string" ? item.value : "",
      values,
    });
  }
  return params;
}

function optionLabel(t: TFunction, exists: (key: string) => boolean, name: string, value?: string): string {
  const key = value === undefined ? `printers.opt.${name}` : `printers.opt.${name}_${value}`;
  return exists(key) ? t(key) : (value ?? name);
}

export function printerRows(body: unknown): PrinterRow[] {
  const list = Array.isArray(body) ? body : [];
  const rows: PrinterRow[] = [];
  for (const item of list) {
    if (!isRecord(item) || typeof item.id !== "string" || item.id === "") continue;
    if (item.available !== true) continue;
    rows.push({
      id: item.id,
      name: typeof item.name === "string" ? item.name : item.id,
      hidden: item.hidden === true,
      default: item.default === true,
      available: true,
      zone: typeof item.zone === "string" ? item.zone : "",
      category: typeof item.category === "string" ? item.category : "",
      print_cert: item.print_cert === true,
      kind: typeof item.kind === "string" ? item.kind : "",
      params: printerParams(item.params),
    });
  }
  return rows;
}

function mergeRows(current: Draft[], incoming: PrinterRow[], dirty: Set<string>): Draft[] {
  const localDefault = current.some((row) => dirty.has(row.id) && row.default);
  return incoming.map((row) => {
    const edited = current.find((item) => item.id === row.id);
    if (edited && dirty.has(row.id)) {
      return { ...edited, available: row.available, kind: row.kind, params: row.params };
    }
    return {
      ...row,
      default: localDefault ? false : row.default,
      saved: edited?.saved ?? false,
    };
  });
}

function EyeIcon({ off }: { off: boolean }) {
  return (
    <svg viewBox="0 0 24 24" className="h-5 w-5" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1.8">
      <path d="M2 12s3.5-6 10-6 10 6 10 6-3.5 6-10 6S2 12 2 12z" />
      <circle cx="12" cy="12" r="2.5" />
      {off ? <path d="M4 4l16 16" /> : null}
    </svg>
  );
}

export default function Printers() {
  const { t, i18n } = useTranslation();
  const catalog = useDeskCatalog();
  const [rows, setRows] = useState<Draft[]>([]);
  const [dirty, setDirty] = useState<Set<string>>(() => new Set());
  const [optionsFor, setOptionsFor] = useState<string | null>(null);
  const [optionDraft, setOptionDraft] = useState<Record<string, string>>({});

  useEffect(() => {
    const client = new FormClient();
    let stop = false;
    const load = async () => {
      try {
        const body = await client.printers();
        if (stop) return;
        setRows((current) => mergeRows(current, printerRows(body), dirty));
      } catch {
        // The next poll asks again.
      }
    };
    void load();
    const timer = setInterval(() => void load(), PRINTER_POLL_MS);
    return () => {
      stop = true;
      clearInterval(timer);
    };
  }, [dirty]);

  function edit(id: string, patch: Partial<PrinterRow>) {
    setDirty((current) => new Set(current).add(id));
    setRows((current) =>
      current.map((row) => {
        if (patch.default === true) {
          if (row.id === id) return { ...row, ...patch, default: true, saved: false };
          return { ...row, default: false, saved: false };
        }
        if (row.id !== id) return row;
        return { ...row, ...patch, saved: false };
      }),
    );
  }

  async function save(row: Draft) {
    const client = new FormClient();
    await client.patchPrinter(row.id, {
      name: row.name,
      hidden: row.hidden,
      default: row.default,
      zone: row.zone,
      category: row.category,
      print_cert: row.print_cert,
    });
    setDirty((current) => {
      const next = new Set(current);
      next.delete(row.id);
      return next;
    });
    setRows((current) => current.map((item) => (item.id === row.id ? { ...item, saved: true } : item)));
  }

  function openOptions(row: Draft) {
    const draft: Record<string, string> = {};
    for (const param of row.params) draft[param.name] = param.value;
    setOptionDraft(draft);
    setOptionsFor(row.id);
  }

  async function saveOptions(row: Draft) {
    const client = new FormClient();
    const body = await client.patchPrinter(row.id, { options: optionDraft });
    const params = printerParams(isRecord(body) ? body.params : undefined);
    setRows((current) =>
      current.map((item) => (item.id === row.id ? { ...item, params: params.length > 0 ? params : item.params } : item)),
    );
    setOptionsFor(null);
  }

  const optionRow = rows.find((row) => row.id === optionsFor) ?? null;

  return (
    <ScreenFrame screen="printers" title={t("screen.printers")} className="flex flex-col gap-2 px-4 py-3">
      {rows.length === 0 ? <p>{t("printers.empty")}</p> : null}
      <ul className="flex flex-col gap-2">
        {rows.map((row) => (
          <li key={row.id} data-printer={row.id} className="relative rounded-lg border border-slate-300 bg-white px-3 py-2">
            <button
              type="button"
              aria-label={t("printers.show")}
              aria-pressed={!row.hidden}
              className="absolute right-2 top-2 inline-flex h-7 w-7 min-h-0 items-center justify-center rounded-md border border-slate-300 bg-white p-0 text-gray-800"
              onClick={() => edit(row.id, { hidden: !row.hidden })}
            >
              <EyeIcon off={row.hidden} />
            </button>
            <p className="truncate pr-10 text-xs text-gray-500">{row.id}</p>
            <div className="mt-1 flex items-end gap-2">
              <label className="flex w-56 shrink-0 flex-col gap-0.5">
                <span className="text-xs text-gray-500">{t("printers.name")}</span>
                <input className="w-full" value={row.name} onChange={(event) => edit(row.id, { name: event.target.value })} />
              </label>
              <label className="flex w-16 shrink-0 flex-col gap-0.5">
                <span className="text-xs text-gray-500">{t("printers.zone")}</span>
                <input
                  className="w-full"
                  type="number"
                  inputMode="numeric"
                  min={0}
                  step={1}
                  value={zoneDigits(row.zone)}
                  onChange={(event) => {
                    const next = event.target.value;
                    if (next !== "" && !/^\d+$/.test(next)) return;
                    edit(row.id, { zone: next });
                  }}
                />
              </label>
              <label className="flex min-w-0 flex-1 flex-col gap-0.5">
                <span className="text-xs text-gray-500">{t("printers.category")}</span>
                <ClosedSelect
                  label={t("printers.category")}
                  options={[{ id: "", label: t("printers.noCategory") }, ...catalog.categories]}
                  value={row.category}
                  onPick={(id) => edit(row.id, { category: id })}
                />
              </label>
            </div>
            <div className="mt-2 flex w-full flex-wrap items-center gap-x-4 gap-y-1">
              <label className="flex cursor-pointer items-center gap-2">
                <input
                  className="cursor-pointer"
                  type="radio"
                  name="printer-default"
                  checked={row.default}
                  onClick={() => {
                    if (row.default) edit(row.id, { default: false });
                  }}
                  onChange={() => {
                    if (!row.default) edit(row.id, { default: true });
                  }}
                />
                {t("printers.default")}
              </label>
              <label className="flex cursor-pointer items-center gap-2">
                <input
                  className="cursor-pointer"
                  type="checkbox"
                  checked={row.print_cert}
                  onChange={(event) => edit(row.id, { print_cert: event.target.checked })}
                />
                {t("printers.certificate")}
              </label>
              <button
                type="button"
                className="min-h-0 shrink-0 rounded-lg border border-slate-300 bg-white px-3 py-1.5 text-sm font-semibold text-gray-900"
                onClick={() => openOptions(row)}
              >
                {t("printers.options")}
              </button>
              <span className="ml-auto flex shrink-0 items-center gap-2">
                <button type="button" className="min-h-0 px-3 py-1.5 text-sm" onClick={() => void save(row)}>
                  {t("printers.save")}
                </button>
                {row.saved ? <span>{t("printers.saved")}</span> : null}
              </span>
            </div>
          </li>
        ))}
      </ul>
      {optionRow ? (
        <div className="fixed inset-0 z-[80] flex items-center justify-center bg-black/40 px-4" role="dialog" aria-modal="true" aria-labelledby="printer-options-title">
          <form
            className="max-h-[80vh] w-full max-w-lg overflow-auto rounded-lg bg-white p-4 shadow"
            onSubmit={(event) => {
              event.preventDefault();
              if (optionRow.params.length === 0) {
                setOptionsFor(null);
                return;
              }
              void saveOptions(optionRow);
            }}
          >
            <p id="printer-options-title" className="font-medium text-gray-900">
              {optionRow.name}
            </p>
            {optionRow.params.length === 0 ? <p className="mt-3 text-gray-700">{t("printers.noOptions")}</p> : null}
            {optionRow.params.map((param) => (
              <label key={param.name} className="mt-3 flex flex-col gap-1 text-gray-900 sm:flex-row sm:items-center sm:justify-between">
                <span>{optionLabel(t, (key) => i18n.exists(key), param.name)}</span>
                <select
                  className="sm:w-64"
                  aria-label={optionLabel(t, (key) => i18n.exists(key), param.name)}
                  value={optionDraft[param.name] ?? param.value}
                  onChange={(event) => setOptionDraft((current) => ({ ...current, [param.name]: event.target.value }))}
                >
                  {param.values.map((value) => (
                    <option key={value} value={value}>
                      {optionLabel(t, (key) => i18n.exists(key), param.name, value)}
                    </option>
                  ))}
                </select>
              </label>
            ))}
            <div className="mt-4 flex justify-end gap-2">
              <button type="button" className="min-h-0 px-3 py-1.5 text-sm" onClick={() => setOptionsFor(null)}>
                {t("settings.cancel")}
              </button>
              <button type="submit" className="min-h-0 px-3 py-1.5 text-sm">
                {t("settings.ok")}
              </button>
            </div>
          </form>
        </div>
      ) : null}
    </ScreenFrame>
  );
}
