import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router";

import { FormClient } from "../form/client";
import { useDeskCatalog, type DeskOption } from "../shell/catalog";
import { openVisitor, visitorHref } from "../shell/open-visitor";
import { MultiSelect } from "../picker";

const PAGE = 20;

type Row = {
  uid: string;
  email: string;
  name: string;
  surname: string;
  company: string;
  category: string;
  prints: number;
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function text(value: unknown): string {
  if (typeof value === "string") return value;
  if (typeof value === "number") return String(value);
  return "";
}

function categoryLabel(value: unknown, options: DeskOption[]): string {
  const raw = text(value);
  if (!raw) return "";
  return options.find((item) => item.id === raw)?.label ?? raw;
}

function count(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

type PageItem = { kind: "page"; page: number } | { kind: "gap" };

function pagerItems(offset: number, total: number): { current: number; items: PageItem[] } {
  const pageCount = Math.max(1, Math.ceil(total / PAGE));
  const current = Math.min(pageCount, Math.floor(offset / PAGE) + 1);
  const start = Math.max(1, current - 2);
  const end = Math.min(pageCount, current + 2);
  const shown = new Set<number>([1, pageCount]);
  for (let page = start; page <= end; page += 1) shown.add(page);
  const ordered = [...shown].sort((left, right) => left - right);
  const items: PageItem[] = [];
  for (const [index, page] of ordered.entries()) {
    const previous = ordered[index - 1];
    if (previous !== undefined && page - previous > 1) items.push({ kind: "gap" });
    items.push({ kind: "page", page });
  }
  return { current, items };
}

function rowTone(index: number, prints: number, picked: boolean): string {
  if (picked) return "cursor-pointer border-b border-sky-300 bg-sky-200";
  const odd = index % 2 === 1;
  const pointer = "cursor-pointer ";
  if (prints > 0) return pointer + (odd ? "border-b border-slate-200 bg-amber-100" : "border-b border-slate-200 bg-amber-50");
  return pointer + (odd ? "border-b border-slate-200 bg-slate-50" : "border-b border-slate-200 bg-white");
}

function maskedEmail(email: string): string {
  const at = email.indexOf("@");
  const head = at < 0 ? email : email.slice(0, at);
  const tail = at < 0 ? "" : email.slice(at);
  const shown = [...head].slice(0, 2).join("");
  return shown + "***" + tail;
}

function rowsOf(body: unknown, options: DeskOption[]): { total: number; rows: Row[] } {
  if (!isRecord(body) || !Array.isArray(body.rows)) return { total: 0, rows: [] };
  const rows = body.rows.filter(isRecord).map((row, index) => ({
    uid: text(row.uid) || text(row.id) || String(index),
    email: text(row.email),
    name: text(row.name),
    surname: text(row.surname),
    company: text(row.c_name),
    category: categoryLabel(row.category, options),
    prints: count(row.print_count),
  }));
  const total = typeof body.total === "number" ? body.total : rows.length;
  return { total, rows };
}

export default function Visitors() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const catalog = useDeskCatalog();
  const client = useMemo(() => new FormClient(), []);
  const [search, setSearch] = useState("");
  const [filter, setFilter] = useState<string[]>([]);
  const [offset, setOffset] = useState(0);
  const [rows, setRows] = useState<Row[]>([]);
  const [total, setTotal] = useState(0);
  const [checked, setChecked] = useState<string[]>([]);
  const [allMatches, setAllMatches] = useState(false);
  const pageBox = useRef<HTMLInputElement>(null);
  const [picked, setPicked] = useState("");
  const [arrived, setArrived] = useState(false);
  const [certificate, setCertificate] = useState(false);
  const pageStart = offset === 0;
  const pageEnd = offset + PAGE >= total;
  const pages = pagerItems(offset, total);
  const filters = useMemo(
    () => [{ id: "", label: t("visitors.all") }, ...catalog.categories],
    [catalog.categories, t],
  );
  const categoryQuery = filters
    .filter((item) => item.id !== "" && filter.includes(item.id))
    .map((item) => item.id)
    .join(",");
  const canSelectMatches = search.trim() !== "" || categoryQuery !== "";
  const pageIds = rows.map((row) => row.uid);
  const pageAll = pageIds.length > 0 && (allMatches || pageIds.every((id) => checked.includes(id)));
  const pageSome = !pageAll && pageIds.some((id) => allMatches || checked.includes(id));

  useEffect(() => {
    if (pageBox.current) pageBox.current.indeterminate = pageSome;
  }, [pageSome]);

  function clearSelection() {
    setChecked([]);
    setAllMatches(false);
  }

  function showPage(next: number) {
    if (next === offset) return;
    if (!allMatches) setChecked([]);
    setOffset(next);
  }

  useEffect(() => {
    let live = true;
    void client
      .registrations({ q: search, category: categoryQuery, limit: PAGE, offset })
      .then((body) => {
        if (!live) return;
        const page = rowsOf(body, catalog.categories);
        setRows(page.rows);
        setTotal(page.total);
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, [client, search, categoryQuery, offset, catalog.categories]);

  function toggle(id: string) {
    if (allMatches) {
      setAllMatches(false);
      setChecked(pageIds.filter((item) => item !== id));
      return;
    }
    setChecked((current) => (current.includes(id) ? current.filter((item) => item !== id) : [...current, id]));
  }

  function togglePage() {
    setAllMatches(false);
    setChecked(pageAll ? [] : pageIds);
  }

  function pickRow(uid: string) {
    if (picked === uid) {
      openVisitor(uid);
      void navigate(visitorHref(uid));
      return;
    }
    setPicked(uid);
  }

  function toggleMatches() {
    if (allMatches) {
      clearSelection();
      return;
    }
    setAllMatches(true);
    setChecked(pageIds);
  }

  return (
    <main data-screen="visitors" data-testid="frame" className="flex w-full flex-col pb-bar">
      <div className="flex flex-col gap-2 px-3 pt-1 pb-2">
        <div className="grid grid-cols-[minmax(0,1fr)_minmax(0,1fr)_auto] items-center gap-x-3 gap-y-1">
          <label className="contents">
            <span className="col-start-1 row-start-1 text-xs font-medium text-gray-500">{t("desk.search")}</span>
            <div className="relative col-start-1 row-start-2 min-w-0">
              <input
                className={search ? "w-full pr-8" : "w-full"}
                aria-label={t("desk.search")}
                value={search}
                onChange={(event) => {
                  setSearch(event.target.value);
                  setOffset(0);
                  clearSelection();
                }}
              />
              {search ? (
                <button type="button" data-testid="clear-search" aria-label={t("desk.clearSearch")} onClick={() => { setSearch(""); setOffset(0); clearSelection(); }}>
                  ×
                </button>
              ) : null}
            </div>
          </label>
          <label className="contents">
            <span className="col-start-2 row-start-1 text-xs font-medium text-gray-500">{t("desk.category")}</span>
            <div className="col-start-2 row-start-2 min-w-0">
              <MultiSelect
                label={t("desk.category")}
                options={filters}
                value={filter}
                none=""
                onChange={(ids) => {
                  setFilter(ids);
                  setOffset(0);
                  clearSelection();
                }}
              />
            </div>
          </label>
          <button type="button" className="col-start-3 row-start-2" data-testid="select-all" aria-pressed={allMatches} disabled={!canSelectMatches} onClick={toggleMatches}>
            {t("visitors.matches")}
          </button>
        </div>
        <table className="w-full table-fixed border-collapse text-left text-sm">
          <thead>
            <tr className="border-b border-slate-300 text-gray-500">
              <th className="w-11 p-0">
                <label>
                  <input
                    ref={pageBox}
                    type="checkbox"
                    aria-label={t("visitors.page")}
                    checked={pageAll}
                    disabled={pageIds.length === 0}
                    onChange={togglePage}
                  />
                </label>
              </th>
              <th className="w-[16%] px-2 py-0.5 font-medium">{t("visitors.name")}</th>
              <th className="w-[18%] px-2 py-0.5 font-medium">{t("visitors.surname")}</th>
              <th className="px-2 py-0.5 font-medium">{t("visitors.company")}</th>
              <th className="w-[18%] px-2 py-0.5 font-medium">{t("desk.category")}</th>
              <th className="w-12 py-0.5" />
            </tr>
          </thead>
          <tbody>
            {rows.map((row, index) => (
              <tr
                key={row.uid}
                aria-selected={picked === row.uid}
                className={rowTone(index, row.prints, picked === row.uid)}
                onClick={() => pickRow(row.uid)}
              >
                <td className="p-0" onClick={(event) => event.stopPropagation()}>
                  <label>
                    <input
                      type="checkbox"
                      aria-label={`${row.name} ${row.surname}`.trim() || row.uid}
                      checked={allMatches || checked.includes(row.uid)}
                      onChange={() => toggle(row.uid)}
                    />
                  </label>
                </td>
                <td className="truncate px-2 py-0.5">{row.name}</td>
                <td className="truncate px-2 py-0.5">{row.surname}</td>
                <td className={row.company.trim() ? "truncate px-2 py-0.5" : "truncate px-2 py-0.5 text-gray-400"}>{row.company.trim() || maskedEmail(row.email)}</td>
                <td className="truncate px-2 py-0.5">{row.category}</td>
                <td className="py-0.5 pr-2 text-right tabular-nums text-gray-600">({row.prints})</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <div data-testid="action-bar" className="fixed inset-x-0 bottom-0 z-20 flex flex-nowrap items-center justify-between gap-2 border-t border-gray-200 bg-white px-3 py-2">
        <div data-testid="pager" className="flex flex-nowrap items-center gap-1">
          <button type="button" aria-label={t("visitors.prev")} disabled={pageStart} onClick={() => showPage(Math.max(0, offset - PAGE))}>
            {"<"}
          </button>
          {pages.items.map((item, index) =>
            item.kind === "gap" ? (
              <span key={`gap-${index}`} className="px-0.5 text-sm text-gray-400">
                ...
              </span>
            ) : (
              <button key={item.page} type="button" aria-current={item.page === pages.current ? "page" : undefined} onClick={() => showPage((item.page - 1) * PAGE)}>
                {item.page}
              </button>
            ),
          )}
          <button type="button" aria-label={t("visitors.next")} disabled={pageEnd} onClick={() => showPage(offset + PAGE)}>
            {">"}
          </button>
        </div>
        <div className="flex flex-nowrap items-center gap-2">
          <button type="button" aria-pressed={arrived} onClick={() => setArrived((value) => !value)}>
            {t("visitors.arrived")}
          </button>
          <button type="button" aria-pressed={certificate} onClick={() => setCertificate((value) => !value)}>
            {t("visitors.certificate")}
          </button>
          <button type="button">{t("visitors.print")}</button>
        </div>
      </div>
    </main>
  );
}
