import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { FormClient } from "../form/client";

type Flags = {
  print_on_save: boolean;
  capitalize_names: boolean;
  show_ticket_status: boolean;
  load_barcodes: boolean;
  double_print: boolean;
  transliterate: boolean;
};

type Pool = {
  catId: number;
  name: string;
  remaining: number;
  total: number;
  min: number;
  add: number;
};

const EMPTY: Flags = {
  print_on_save: false,
  capitalize_names: false,
  show_ticket_status: false,
  load_barcodes: false,
  double_print: false,
  transliterate: false,
};

const SWITCHES: { field: keyof Flags; label: string }[] = [
  { field: "print_on_save", label: "settings.ipad" },
  { field: "capitalize_names", label: "settings.capitalize" },
  { field: "show_ticket_status", label: "settings.tickets" },
  { field: "load_barcodes", label: "settings.loadBarcodes" },
  { field: "double_print", label: "settings.double" },
  { field: "transliterate", label: "settings.translit" },
];

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function flagsOf(body: unknown): Flags {
  if (!isRecord(body)) return { ...EMPTY };
  return {
    print_on_save: body.print_on_save === true,
    capitalize_names: body.capitalize_names === true,
    show_ticket_status: body.show_ticket_status === true,
    load_barcodes: body.load_barcodes === true,
    double_print: body.double_print === true,
    transliterate: body.transliterate === true,
  };
}

function poolName(value: unknown, catId: number): string {
  if (typeof value === "string" && value !== "") return value;
  if (isRecord(value)) {
    const ru = value.ru;
    const en = value.en;
    if (typeof ru === "string" && ru !== "") return ru;
    if (typeof en === "string" && en !== "") return en;
  }
  return String(catId);
}

function countOf(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

export function barcodePools(body: unknown): Pool[] {
  const list = Array.isArray(body) ? body : [];
  const pools: Pool[] = [];
  for (const item of list) {
    if (!isRecord(item) || typeof item.cat_id !== "number") continue;
    pools.push({
      catId: item.cat_id,
      name: poolName(item.name, item.cat_id),
      remaining: countOf(item.remaining),
      total: countOf(item.total),
      min: countOf(item.min),
      add: countOf(item.add),
    });
  }
  return pools;
}

function fill(part: number, total: number): string {
  if (total <= 0 || part <= 0) return "0%";
  return `${Math.min(100, (part / total) * 100)}%`;
}

function whole(value: string): number | null {
  if (!/^\d+$/.test(value)) return null;
  const parsed = Number(value);
  return Number.isSafeInteger(parsed) ? parsed : null;
}

export default function Settings() {
  const { t } = useTranslation();
  const [flags, setFlags] = useState<Flags>(EMPTY);
  const [pools, setPools] = useState<Pool[]>([]);
  const [editing, setEditing] = useState<Pool | null>(null);
  const [minimum, setMinimum] = useState("");
  const [add, setAdd] = useState("");

  useEffect(() => {
    const client = new FormClient();
    let stop = false;
    void client.settings().then(
      (body) => {
        if (!stop) setFlags(flagsOf(body));
      },
      () => undefined,
    );
    void client.barcodes().then(
      (body) => {
        if (!stop) setPools(barcodePools(body));
      },
      () => undefined,
    );
    return () => {
      stop = true;
    };
  }, []);

  function openPool(pool: Pool) {
    setEditing(pool);
    setMinimum(String(pool.min));
    setAdd(String(pool.add));
  }

  async function toggle(field: keyof Flags) {
    const next = !flags[field];
    setFlags((current) => ({ ...current, [field]: next }));
    try {
      const body = await new FormClient().patchSettings({ [field]: next });
      setFlags(flagsOf(body));
    } catch {
      setFlags((current) => ({ ...current, [field]: !next }));
    }
  }

  async function savePool() {
    if (!editing) return;
    const min = whole(minimum);
    const extra = whole(add);
    if (min === null || extra === null) return;
    const catId = editing.catId;
    setEditing(null);
    try {
      const body = await new FormClient().patchBarcodes({ cat_id: catId, min, add: extra });
      setPools(barcodePools(body));
    } catch {
      setPools((current) => current.map((pool) => (pool.catId === catId ? { ...pool, min, add: extra } : pool)));
    }
  }

  return (
    <main data-screen="settings" className="mx-auto flex w-full max-w-3xl flex-col gap-4 px-4 py-4">
      <ul className="flex flex-col gap-2">
        {SWITCHES.map((item) => (
          <li key={item.field}>
            <label className="flex items-center justify-between gap-4 rounded-lg border border-slate-300 bg-white px-4 py-3 text-gray-900">
              <span>{t(item.label)}</span>
              <input
                type="checkbox"
                checked={flags[item.field]}
                onChange={() => {
                  void toggle(item.field);
                }}
              />
            </label>
          </li>
        ))}
      </ul>
      <ul className="flex flex-col gap-3">
        {pools.map((pool) => (
          <li key={pool.catId}>
            <button
              type="button"
              className="min-h-0 w-full rounded-lg border border-slate-300 bg-white px-4 py-3 text-left text-gray-900"
              onClick={() => openPool(pool)}
            >
              <span className="block font-medium">{pool.name}</span>
              <span className="relative mt-2 block h-6 overflow-hidden rounded bg-slate-200">
                <span className="absolute inset-y-0 left-0 bg-sky-400" style={{ width: fill(pool.remaining, pool.total) }} />
                <span className="absolute inset-0 flex items-center justify-center text-sm">
                  {pool.remaining} / {pool.total}
                </span>
              </span>
            </button>
          </li>
        ))}
      </ul>
      <button
        type="button"
        onClick={() => {
          void new FormClient().restartSync().catch(() => undefined);
        }}
      >
        {t("settings.sync")}
      </button>
      {editing ? (
        <div className="fixed inset-0 z-[80] flex items-center justify-center bg-black/40 px-4" role="dialog" aria-modal="true" aria-labelledby="barcode-title">
          <form
            className="w-full max-w-sm rounded-lg bg-white p-4 shadow"
            onSubmit={(event) => {
              event.preventDefault();
              void savePool();
            }}
          >
            <p id="barcode-title" className="font-medium text-gray-900">
              {editing.name}
            </p>
            <label className="mt-3 flex flex-col gap-1 text-gray-900">
              {t("settings.min")}
              <input value={minimum} onChange={(event) => setMinimum(event.target.value)} />
            </label>
            <label className="mt-3 flex flex-col gap-1 text-gray-900">
              {t("settings.add")}
              <input value={add} onChange={(event) => setAdd(event.target.value)} />
            </label>
            <div className="mt-4 flex justify-end gap-2">
              <button type="button" onClick={() => setEditing(null)}>
                {t("settings.cancel")}
              </button>
              <button type="submit">{t("settings.ok")}</button>
            </div>
          </form>
        </div>
      ) : null}
    </main>
  );
}
