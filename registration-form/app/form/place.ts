import type { EnumParents, EnumRow, FormClient } from "./client";
import { type FormState, type PlaceBinding, type Question } from "./engine";
import { type CityHit, cityById, loadCities, searchCities as searchLoadedCities, whenCity } from "./files";

type EnumClient = Pick<FormClient, "enums">;

const FIELDS = {
  country: "country_db",
  region: "region_db",
  city: "city_db",
} as const;

type PlaceName = keyof typeof FIELDS;

export function placeKey(path: string, index: number): string {
  return `${path}#${index}`;
}

type AddressCountryNote = (
  state: FormState,
  client: EnumClient,
  path: string,
  index: number,
  countryId: number,
) => void;

let addressCountryNote: AddressCountryNote | null = null;

export function onAddressCountry(note: AddressCountryNote) {
  addressCountryNote = note;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function primary(rows: unknown[]): Record<string, unknown> | undefined {
  const records = rows.filter(isRecord);
  return records.find((row) => isRecord(row.jvrel_prop) && row.jvrel_prop.primary === true) ?? records[0];
}

function readPath(root: Record<string, unknown>, path: string): unknown {
  let current: unknown = root;
  for (const part of path.split(".")) {
    if (Array.isArray(current)) current = primary(current);
    if (!isRecord(current)) return undefined;
    current = current[part];
  }
  return current;
}

function addressRows(visitor: Record<string, unknown>, path: string): Record<string, unknown>[] {
  const value = readPath(visitor, path);
  if (Array.isArray(value)) return value.filter(isRecord);
  if (isRecord(value)) return [value];
  return [];
}

function enumValue(row: Record<string, unknown>, field: string): number | null {
  const node = row[field];
  if (!isRecord(node)) return null;
  return typeof node.e_val === "number" ? node.e_val : null;
}

function writeEnum(row: Record<string, unknown>, field: string, value: number | null) {
  const node = row[field];
  if (isRecord(node) && "e_val" in node) node.e_val = value;
}

function listDefault(list: EnumRow[], questionDef?: number): number | null {
  const marked = list.find((row) => row.d === true);
  if (marked) return marked.id;
  if (questionDef !== undefined && list.some((row) => row.id === questionDef)) return questionDef;
  if (list.length === 1) return list[0].id;
  return null;
}

function addressPaths(state: FormState): Map<string, Partial<Record<PlaceName, Question>>> {
  const paths = new Map<string, Partial<Record<PlaceName, Question>>>();
  for (const question of Object.values(state.questions)) {
    const name = question.leaf?.enumName;
    if (name !== "country" && name !== "region" && name !== "city") continue;
    const path = question.id.slice(0, question.id.lastIndexOf("."));
    const fields = paths.get(path) ?? {};
    fields[name] = question;
    paths.set(path, fields);
  }
  return paths;
}

function bindingFor(
  state: FormState,
  path: string,
  index: number,
  fields: Partial<Record<PlaceName, Question>>,
): PlaceBinding {
  const key = placeKey(path, index);
  const existing = state.places[key];
  if (existing) return existing;
  const binding: PlaceBinding = { path, index };
  for (const name of ["country", "region", "city"] as const) {
    const question = fields[name];
    if (!question) continue;
    binding[name] = { list: [], error: null, def: question.def, ui: name !== "region" };
  }
  state.places[key] = binding;
  return binding;
}

function rowAt(state: FormState, path: string, index: number): Record<string, unknown> | undefined {
  return addressRows(state.visitor, path)[index];
}

export async function loadPlaces(state: FormState, client: EnumClient) {
  const countries = await client.enums(state.locale, "country", {});
  for (const [path, fields] of addressPaths(state)) {
    const rows = addressRows(state.visitor, path);
    rows.forEach((row, index) => {
      const binding = bindingFor(state, path, index, fields);
      if (!binding.country) return;
      binding.country.list = countries.list;
      binding.country.error = countries.error;
      if (enumValue(row, FIELDS.country) === null && binding.country.def !== undefined) {
        if (countries.list.some((item) => item.id === binding.country?.def)) {
          writeEnum(row, FIELDS.country, binding.country.def);
        }
      }
    });
  }
}

async function loadNamed(
  client: EnumClient,
  locale: string,
  name: PlaceName,
  parents: EnumParents,
  refresh: boolean,
): Promise<{ list: EnumRow[]; error: Error | null }> {
  return client.enums(locale, name, parents, refresh ? { refresh: true } : undefined);
}

async function applyChildren(state: FormState, client: EnumClient, path: string, index: number) {
  const binding = state.places[placeKey(path, index)];
  const row = rowAt(state, path, index);
  if (!binding || !row) return;
  const countryId = enumValue(row, FIELDS.country);
  if (countryId === null || !binding.region) return;

  const regions = await loadNamed(client, state.locale, "region", { country: countryId }, false);
  binding.region.list = regions.list;
  binding.region.error = regions.error;
  const regionId = listDefault(regions.list, binding.region.def);
  writeEnum(row, FIELDS.region, regionId);
  if (regionId === null || !binding.city) return;

  const cities = await loadCities(
    state,
    client,
    countryId,
    state.locale,
    regions.list.map((item) => item.id),
    regionId,
  );
  if (!cities) return;
  binding.city.list = cities.list;
  binding.city.error = cities.error;
  writeEnum(row, FIELDS.city, listDefault(cities.list, binding.city.def));
}

export async function setCountry(
  state: FormState,
  client: EnumClient,
  path: string,
  countryId: number,
  index = 0,
) {
  const fields = addressPaths(state).get(path) ?? {};
  const binding = bindingFor(state, path, index, fields);
  const row = rowAt(state, path, index);
  if (!row || !binding.country) return;
  writeEnum(row, FIELDS.country, countryId);
  await applyChildren(state, client, path, index);
  addressCountryNote?.(state, client, path, index, countryId);
}

export async function setCity(
  state: FormState,
  client: EnumClient,
  path: string,
  cityId: number,
  index = 0,
) {
  const fields = addressPaths(state).get(path) ?? {};
  const binding = bindingFor(state, path, index, fields);
  const row = rowAt(state, path, index);
  if (!row || !binding.city || !binding.region) return;
  const countryId = enumValue(row, FIELDS.country);
  if (countryId === null) return;

  if (binding.region.list.length === 0) {
    const regions = await loadNamed(client, state.locale, "region", { country: countryId }, false);
    binding.region.list = regions.list;
    binding.region.error = regions.error;
  }

  const first = enumValue(row, FIELDS.region) ?? listDefault(binding.region.list, binding.region.def);
  loadCities(
    state,
    client,
    countryId,
    state.locale,
    binding.region.list.map((item) => item.id),
    first,
  );
  const hit = await whenCity(state, countryId, state.locale, cityId);
  if (!hit) return;
  const cities = await loadCities(state, client, countryId, state.locale, [hit.regionId], hit.regionId);
  if (!cities) return;
  binding.city.list = cities.list;
  binding.city.error = cities.error;
  writeEnum(row, FIELDS.city, cityId);
  writeEnum(row, FIELDS.region, hit.regionId);
}

export async function openCitySearch(state: FormState, client: EnumClient, path: string, index = 0) {
  await loadPlaces(state, client);
  const row = rowAt(state, path, index);
  if (!row) return;
  const countryId = enumValue(row, FIELDS.country);
  if (countryId === null || countryId < 0) return;
  const fields = addressPaths(state).get(path) ?? {};
  const binding = bindingFor(state, path, index, fields);
  if (!binding.region) return;
  if (binding.region.list.length === 0) {
    const regions = await loadNamed(client, state.locale, "region", { country: countryId }, false);
    binding.region.list = regions.list;
    binding.region.error = regions.error;
  }
  const stored = enumValue(row, FIELDS.region);
  const first = stored !== null && stored >= 0 ? stored : listDefault(binding.region.list, binding.region.def);
  void loadCities(
    state,
    client,
    countryId,
    state.locale,
    binding.region.list.map((item) => item.id),
    first,
  );
}

function orderCities(hits: CityHit[], def: number | undefined): CityHit[] {
  return hits
    .map((hit, index) => ({ hit, index }))
    .sort((left, right) => {
      if (left.hit.weight !== right.hit.weight) return right.hit.weight - left.hit.weight;
      if (def !== undefined) {
        if (left.hit.id === def) return -1;
        if (right.hit.id === def) return 1;
      }
      return left.index - right.index;
    })
    .map((item) => item.hit);
}

function rowName(row: EnumRow, locale: string): string {
  if (typeof row.v === "string") return row.v;
  const name = locale === "en" ? row.v.en || row.v.ru : row.v.ru || row.v.en;
  return name ?? "";
}

export function countryValue(state: FormState, path: string, index = 0): number | null {
  const row = rowAt(state, path, index);
  if (!row) return null;
  const countryId = enumValue(row, FIELDS.country);
  if (countryId === null || countryId < 0) return null;
  return countryId;
}

function listCountries(state: FormState, path: string, index = 0): { id: number; name: string }[] {
  const list = state.places[placeKey(path, index)]?.country?.list ?? [];
  return list.flatMap((item) => {
    const name = rowName(item, state.locale);
    if (!name) return [];
    return [{ id: item.id, name }];
  });
}

function startsWord(name: string, needle: string, locale: string): boolean {
  const text = name.toLocaleLowerCase(locale);
  let from = 0;
  while (from <= text.length - needle.length) {
    const at = text.indexOf(needle, from);
    if (at < 0) return false;
    if (at === 0 || !/\p{L}/u.test(text.charAt(at - 1))) return true;
    from = at + 1;
  }
  return false;
}

export function searchCountries(
  state: FormState,
  path: string,
  query: string,
  index = 0,
): { id: number; name: string }[] {
  const needle = query.trim().toLocaleLowerCase(state.locale);
  const current = countryValue(state, path, index);
  return listCountries(state, path, index)
    .filter((item) => !needle || startsWord(item.name, needle, state.locale))
    .sort((left, right) => {
      if (needle || current === null) return 0;
      if (left.id === current) return -1;
      if (right.id === current) return 1;
      return 0;
    })
    .slice(0, 12);
}

export function countryLabel(state: FormState, path: string, index = 0): string {
  const row = rowAt(state, path, index);
  if (!row) return "";
  const countryId = enumValue(row, FIELDS.country);
  if (countryId === null || countryId < 0) return "";
  const found = state.places[placeKey(path, index)]?.country?.list.find((item) => item.id === countryId);
  return found ? rowName(found, state.locale) : "";
}

export function cityLabel(state: FormState, path: string, index = 0): string {
  const row = rowAt(state, path, index);
  if (!row) return "";
  const countryId = enumValue(row, FIELDS.country);
  const cityId = enumValue(row, FIELDS.city);
  if (countryId === null || cityId === null || cityId < 0) return "";
  return cityById(state, countryId, state.locale, cityId)?.name ?? "";
}

export function searchCities(state: FormState, path: string, query: string, index = 0): CityHit[] {
  const row = rowAt(state, path, index);
  if (!row) return [];
  const countryId = enumValue(row, FIELDS.country);
  if (countryId === null) return [];
  const def = state.places[placeKey(path, index)]?.city?.def ?? addressPaths(state).get(path)?.city?.def;
  if (!query.trim()) {
    if (def === undefined) return [];
    const hit = cityById(state, countryId, state.locale, def);
    return hit ? [hit] : [];
  }
  return orderCities(searchLoadedCities(state, countryId, state.locale, query), def);
}

export async function refreshPlace(
  state: FormState,
  client: EnumClient,
  path: string,
  index: number,
  name: PlaceName,
) {
  const binding = state.places[placeKey(path, index)];
  const row = rowAt(state, path, index);
  const control = binding?.[name];
  if (!binding || !row || !control) return;
  const countryId = enumValue(row, FIELDS.country);
  const regionId = enumValue(row, FIELDS.region);
  const parents: EnumParents =
    name === "country" ? {} : name === "region" ? { country: countryId ?? undefined } : { country: countryId ?? undefined, region: regionId ?? undefined };
  if (name !== "country" && countryId === null) return;
  if (name === "city" && regionId === null) return;
  const previous = control.list;
  try {
    const result = await loadNamed(client, state.locale, name, parents, true);
    if (result.error) {
      control.list = previous;
      control.error = result.error;
      return;
    }
    control.list = result.list;
    control.error = null;
  } catch (caught) {
    control.list = previous;
    control.error = caught instanceof Error ? caught : new Error(String(caught));
  }
}
