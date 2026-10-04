import type { EnumRow, FormClient } from "./client";
import type { FormState } from "./engine";

type EnumClient = Pick<FormClient, "enums">;

export const FILE_LIMIT = 10;

export type CityHit = {
  id: number;
  regionId: number;
  name: string;
  weight: number;
};

type CityFile = { list: EnumRow[]; error: Error | null };

type Gate = {
  active: number;
  waiters: Array<() => void>;
  idle: Array<() => void>;
};

type Waiter = {
  key: string;
  cityId: number;
  resolve: (hit: CityHit | undefined) => void;
};

type Catalog = {
  hits: Map<string, Map<number, CityHit>>;
  names: Map<string, CityHit[]>;
  files: Map<string, Promise<CityFile>>;
  pending: Map<string, number>;
  waiting: Waiter[];
};

const gates = new WeakMap<FormState, Gate>();
const catalogs = new WeakMap<FormState, Catalog>();
const listeners = new WeakMap<FormState, Set<() => void>>();

export function watchCities(state: FormState, onChange: () => void): () => void {
  const existing = listeners.get(state);
  const set = existing ?? new Set<() => void>();
  if (!existing) listeners.set(state, set);
  set.add(onChange);
  return () => {
    set.delete(onChange);
  };
}

function gateFor(state: FormState): Gate {
  const existing = gates.get(state);
  if (existing) return existing;
  const created: Gate = { active: 0, waiters: [], idle: [] };
  gates.set(state, created);
  return created;
}

function catalogFor(state: FormState): Catalog {
  const existing = catalogs.get(state);
  if (existing) return existing;
  const created: Catalog = {
    hits: new Map(),
    names: new Map(),
    files: new Map(),
    pending: new Map(),
    waiting: [],
  };
  catalogs.set(state, created);
  return created;
}

function countryKey(locale: string, countryId: number): string {
  return `${locale}|${countryId}`;
}

function fileKey(locale: string, countryId: number, regionId: number): string {
  return `${locale}|${countryId}|${regionId}`;
}

function release(gate: Gate) {
  gate.active -= 1;
  const next = gate.waiters.shift();
  if (next) {
    next();
    return;
  }
  if (gate.active === 0) {
    const pending = gate.idle.splice(0);
    for (const resolve of pending) resolve();
  }
}

export function schedule<T>(state: FormState, run: () => Promise<T>): Promise<T> {
  const gate = gateFor(state);
  return new Promise((resolve, reject) => {
    const start = () => {
      gate.active += 1;
      run().then(resolve, reject).finally(() => release(gate));
    };
    if (gate.active < FILE_LIMIT) start();
    else gate.waiters.push(start);
  });
}

export function settleFiles(state: FormState): Promise<void> {
  const gate = gateFor(state);
  if (gate.active === 0 && gate.waiters.length === 0) return Promise.resolve();
  return new Promise((resolve) => gate.idle.push(resolve));
}

function label(row: EnumRow, locale: string): string {
  if (typeof row.v === "string") return row.v;
  if (locale === "ru" || locale === "en") return row.v[locale] ?? "";
  return "";
}

function publish(state: FormState, locale: string, countryId: number) {
  const catalog = catalogFor(state);
  const key = countryKey(locale, countryId);
  const found = catalog.hits.get(key);
  const done = (catalog.pending.get(key) ?? 0) === 0;
  catalog.waiting = catalog.waiting.filter((waiter) => {
    if (waiter.key !== key) return true;
    const hit = found?.get(waiter.cityId);
    if (hit) {
      waiter.resolve(hit);
      return false;
    }
    if (done) {
      waiter.resolve(undefined);
      return false;
    }
    return true;
  });
  for (const listener of listeners.get(state) ?? []) listener();
}

function ensureCityFile(
  state: FormState,
  client: EnumClient,
  countryId: number,
  locale: string,
  regionId: number,
): Promise<CityFile> {
  const catalog = catalogFor(state);
  const key = fileKey(locale, countryId, regionId);
  const existing = catalog.files.get(key);
  if (existing) return existing;
  const group = countryKey(locale, countryId);
  catalog.pending.set(group, (catalog.pending.get(group) ?? 0) + 1);
  const task = schedule(state, () => client.enums(locale, "city", { country: countryId, region: regionId }))
    .then((result) => {
      const hits = catalog.hits.get(group) ?? new Map<number, CityHit>();
      const names = catalog.names.get(group) ?? [];
      catalog.hits.set(group, hits);
      catalog.names.set(group, names);
      for (const row of result.list) {
        if (hits.has(row.id)) continue;
        const hit = { id: row.id, regionId, name: label(row, locale), weight: weightOf(row) };
        hits.set(row.id, hit);
        names.push(hit);
      }
      return { list: result.list, error: result.error };
    })
    .catch((caught: unknown) => {
      const error = caught instanceof Error ? caught : new Error(String(caught));
      return { list: [], error };
    })
    .finally(() => {
      catalog.pending.set(group, (catalog.pending.get(group) ?? 1) - 1);
      publish(state, locale, countryId);
    });
  catalog.files.set(key, task);
  return task;
}

export function loadCities(
  state: FormState,
  client: EnumClient,
  countryId: number,
  locale: string,
  regionIds: number[],
  firstRegionId: number | null,
): Promise<CityFile | undefined> {
  const ordered =
    firstRegionId === null ? regionIds : [firstRegionId, ...regionIds.filter((id) => id !== firstRegionId)];
  const tasks = ordered.map((regionId) => ensureCityFile(state, client, countryId, locale, regionId));
  return tasks[0] ?? Promise.resolve(undefined);
}

export function whenCity(
  state: FormState,
  countryId: number,
  locale: string,
  cityId: number,
): Promise<CityHit | undefined> {
  const catalog = catalogFor(state);
  const key = countryKey(locale, countryId);
  const found = catalog.hits.get(key)?.get(cityId);
  if (found) return Promise.resolve(found);
  if ((catalog.pending.get(key) ?? 0) === 0) return Promise.resolve(undefined);
  return new Promise((resolve) => {
    catalog.waiting.push({ key, cityId, resolve });
  });
}

export function cityById(state: FormState, countryId: number, locale: string, cityId: number): CityHit | undefined {
  return catalogFor(state).hits.get(countryKey(locale, countryId))?.get(cityId);
}

function weightOf(row: EnumRow): number {
  if (typeof row.weight === "number") return row.weight;
  if (row.d) return 10;
  return 1;
}

function isLetter(char: string): boolean {
  return /\p{L}/u.test(char);
}

function startsWord(name: string, needle: string, locale: string): boolean {
  const text = name.toLocaleLowerCase(locale);
  let from = 0;
  while (from <= text.length - needle.length) {
    const at = text.indexOf(needle, from);
    if (at < 0) return false;
    if (at === 0 || !isLetter(text.charAt(at - 1))) return true;
    from = at + 1;
  }
  return false;
}

export function searchCities(state: FormState, countryId: number, locale: string, query: string): CityHit[] {
  const names = catalogFor(state).names.get(countryKey(locale, countryId)) ?? [];
  const needle = query.trim().toLocaleLowerCase(locale);
  if (!needle) return names.slice();
  return names.filter((hit) => startsWord(hit.name, needle, locale));
}
