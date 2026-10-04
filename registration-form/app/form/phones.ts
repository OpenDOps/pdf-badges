import type { EnumRow, FormClient } from "./client";
import type { FormState } from "./engine";
import { schedule } from "./files";
import { onAddressCountry, placeKey } from "./place";

const PHONE_KEYS = ["contact_phones", "phones_faxes", "faxes"] as const;

type PhoneBinding = {
  countryId: number | null;
  regionId: number | null;
  callingCode: string;
  countryMasks: string;
  mask: string;
  chosen: boolean;
  epoch: number;
  regionMasks: Map<number, EnumRow[]>;
  regionTasks: Map<number, Promise<void>>;
};

const bindings = new WeakMap<FormState, Map<string, PhoneBinding>>();

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

function addressRow(state: FormState, path: string, index: number): Record<string, unknown> | undefined {
  const value = readPath(state.visitor, path);
  if (!Array.isArray(value)) return undefined;
  const row = value[index];
  return isRecord(row) ? row : undefined;
}

function bindingMap(state: FormState): Map<string, PhoneBinding> {
  const existing = bindings.get(state);
  if (existing) return existing;
  const created = new Map<string, PhoneBinding>();
  bindings.set(state, created);
  return created;
}

function bindingFor(state: FormState, addressPath: string, index: number): PhoneBinding {
  const key = placeKey(addressPath, index);
  const map = bindingMap(state);
  const existing = map.get(key);
  if (existing) return existing;
  const created: PhoneBinding = {
    countryId: null,
    regionId: null,
    callingCode: "",
    countryMasks: "",
    mask: "",
    chosen: false,
    epoch: 0,
    regionMasks: new Map(),
    regionTasks: new Map(),
  };
  map.set(key, created);
  return created;
}

function phoneArray(row: Record<string, unknown>, create: boolean): Record<string, unknown>[] | undefined {
  for (const key of PHONE_KEYS) {
    const value = row[key];
    if (Array.isArray(value)) return value as Record<string, unknown>[];
  }
  if (!create) return undefined;
  const created: Record<string, unknown>[] = [];
  row.contact_phones = created;
  return created;
}

function text(value: EnumRow["v"]): string {
  return typeof value === "string" ? value : "";
}

function paren(mask: string): string {
  const match = /\(([^)]*)\)/.exec(mask);
  return match?.[1] ?? "";
}

function areaWidth(mask: string): number {
  return paren(mask).replace(/[^d\d]/g, "").length;
}

function cityCode(mask: string): string {
  const inside = paren(mask);
  return /^\d+$/.test(inside) ? inside : "";
}

function countryPatterns(masks: string): string[] {
  return masks
    .split("; ")
    .map((part) => part.trim())
    .filter(Boolean);
}

function regionCodes(rows: EnumRow[]): string[] {
  const codes: string[] = [];
  for (const row of rows) {
    const value = text(row.v);
    if (!value) continue;
    for (const part of value.split("; ")) {
      const code = cityCode(part.trim());
      if (code) codes.push(code);
    }
  }
  return codes;
}

function chooseCountry(masks: string, national: string, codes: string[]): string {
  const patterns = countryPatterns(masks);
  if (patterns.length === 0) return "";
  for (let length = national.length; length >= 0; length -= 1) {
    if (length === 0) return patterns[0];
    const prefix = national.slice(0, length);
    const widths = new Set<number>();
    for (const code of codes) {
      if (code.startsWith(prefix) && code.length > prefix.length) widths.add(code.length);
    }
    if (widths.size !== 1) continue;
    const width = [...widths][0];
    const match = patterns.find((part) => areaWidth(part) === width);
    if (match) return match;
  }
  return patterns[0];
}

function chooseCity(rows: EnumRow[], national: string): { id: number; mask: string } | null {
  let bestLength = 0;
  let best: { id: number; mask: string } | null = null;
  for (const row of rows) {
    const value = text(row.v);
    if (!value) continue;
    for (const part of value.split("; ")) {
      const code = cityCode(part.trim());
      if (!code || national.length < code.length || !national.startsWith(code)) continue;
      if (code.length <= bestLength) continue;
      bestLength = code.length;
      best = { id: row.id, mask: part.trim() };
    }
  }
  return best;
}

function loadedCallingCode(state: FormState, addressPath: string, index: number): string {
  return bindings.get(state)?.get(placeKey(addressPath, index))?.callingCode ?? "";
}

function phoneAddress(state: FormState, questionId: string): { path: string; index: number } | undefined {
  const match = /^(.*)\.(?:contact_phones|phones_faxes|faxes)\.str_number$/.exec(questionId);
  const path = match?.[1];
  if (!path) return undefined;
  const value = readPath(state.visitor, path);
  if (!Array.isArray(value)) return { path, index: 0 };
  const chosen = primary(value);
  const index = chosen ? value.indexOf(chosen) : 0;
  return { path, index: index < 0 ? 0 : index };
}

export function phoneHasNationalDigits(state: FormState, questionId: string, value: unknown): boolean {
  if (typeof value !== "string") return false;
  const trimmed = value.trim();
  if (trimmed === "") return false;
  const address = phoneAddress(state, questionId);
  const callingCode = address ? loadedCallingCode(state, address.path, address.index) : "";
  return nationalDigits(trimmed, callingCode).length > 0;
}

function nationalDigits(stored: string, callingCode: string): string {
  if (callingCode && stored.startsWith(callingCode)) return stored.slice(callingCode.length).replace(/\D/g, "");
  const digits = stored.replace(/\D/g, "");
  const codeDigits = callingCode.replace(/\D/g, "");
  if (codeDigits && digits.startsWith(codeDigits)) return digits.slice(codeDigits.length);
  return digits;
}

function rewriteNumbers(rows: Record<string, unknown>[] | undefined, previousCode: string, nextCode: string) {
  if (!rows) return;
  for (const row of rows) {
    if (typeof row.str_number !== "string") continue;
    const national = nationalDigits(row.str_number, previousCode);
    row.str_number = nextCode ? nextCode + national : national;
  }
}

function rowValue(list: EnumRow[], id: number): string {
  return text(list.find((row) => row.id === id)?.v ?? "");
}

function fetchRegion(
  state: FormState,
  client: Pick<FormClient, "enums">,
  addressPath: string,
  index: number,
  countryId: number,
  regionId: number,
): Promise<void> {
  const binding = bindingFor(state, addressPath, index);
  const existing = binding.regionTasks.get(regionId);
  if (existing) return existing;
  const epoch = binding.epoch;
  const task = schedule(state, () =>
    client.enums(state.locale, "city_phone_mask", { country: countryId, region: regionId }),
  )
    .then((result) => {
      const current = bindingFor(state, addressPath, index);
      if (current.epoch !== epoch || current.countryId !== countryId) return;
      current.regionMasks.set(regionId, result.list);
      applyMask(state, addressPath, index);
    })
    .catch(() => {
      const current = bindingFor(state, addressPath, index);
      if (current.epoch !== epoch || current.countryId !== countryId) return;
      current.regionMasks.set(regionId, []);
      applyMask(state, addressPath, index);
    });
  binding.regionTasks.set(regionId, task);
  return task;
}

function loadRegionFiles(
  state: FormState,
  client: Pick<FormClient, "enums">,
  addressPath: string,
  index: number,
  countryId: number,
  regionIds: number[],
  current: number | null,
) {
  const ordered = current === null ? regionIds : [current, ...regionIds.filter((id) => id !== current)];
  for (const regionId of ordered) {
    void fetchRegion(state, client, addressPath, index, countryId, regionId);
  }
}

function preferredRegion(list: EnumRow[]): number | null {
  return list.find((row) => row.d)?.id ?? list[0]?.id ?? null;
}

export async function setPhoneCountry(
  state: FormState,
  client: Pick<FormClient, "enums">,
  addressPath: string,
  countryId: number | null,
  index = 0,
) {
  await applyPhoneCountry(state, client, addressPath, index, countryId, true);
}

async function applyPhoneCountry(
  state: FormState,
  client: Pick<FormClient, "enums">,
  addressPath: string,
  index: number,
  countryId: number | null,
  choose: boolean,
) {
  const row = addressRow(state, addressPath, index);
  if (!row) return;
  const binding = bindingFor(state, addressPath, index);
  if (choose) binding.chosen = true;
  const previousId = binding.countryId;
  const previousCode = binding.callingCode;
  if (previousId !== countryId) {
    binding.epoch += 1;
    binding.regionMasks.clear();
    binding.regionTasks.clear();
  }
  const ticket = binding.epoch;
  if (countryId === null) {
    if (choose && previousId !== null) rewriteNumbers(phoneArray(row, false), previousCode, "");
    binding.countryId = null;
    binding.regionId = null;
    binding.callingCode = "";
    binding.countryMasks = "";
    binding.mask = "";
    return;
  }

  const [codes, masks, regions] = await Promise.all([
    client.enums(state.locale, "phone_code", {}),
    client.enums(state.locale, "phone_mask", {}),
    client.enums(state.locale, "region", { country: countryId }),
  ]);
  if (binding.epoch !== ticket) return;
  if (!choose) {
    const phones = phoneArray(row, false);
    const phone = phones?.[0];
    if (phone && (phone.str_number === "" || phone.str_number === previousCode)) phone.str_number = "";
  }
  const callingCode = rowValue(codes.list, countryId);
  const countryMasks = rowValue(masks.list, countryId);
  if (choose && previousId !== null && previousId !== countryId) {
    rewriteNumbers(phoneArray(row, false), previousCode, callingCode);
  }
  binding.countryId = countryId;
  binding.callingCode = callingCode;
  binding.countryMasks = countryMasks;
  binding.regionId = preferredRegion(regions.list);
  applyMask(state, addressPath, index);
  loadRegionFiles(
    state,
    client,
    addressPath,
    index,
    countryId,
    regions.list.map((item) => item.id),
    binding.regionId,
  );
}

export async function loadPhoneRegion(
  state: FormState,
  client: Pick<FormClient, "enums">,
  addressPath: string,
  regionId: number,
  index = 0,
) {
  const binding = bindingFor(state, addressPath, index);
  binding.regionId = regionId;
  applyMask(state, addressPath, index);
  if (binding.countryId === null) return;
  await fetchRegion(state, client, addressPath, index, binding.countryId, regionId);
}

export function phoneCountryMasks(state: FormState, addressPath: string, index = 0): string {
  return bindingFor(state, addressPath, index).countryMasks;
}

export function phoneCallingCode(state: FormState, addressPath: string, index = 0): string {
  return bindingFor(state, addressPath, index).callingCode;
}

function ensurePhone(rows: Record<string, unknown>[]): Record<string, unknown> {
  const existing = rows.find(isRecord);
  if (existing) return existing;
  const created = { jvlink_id: -1, jvrel_prop: { primary: true }, str_number: "", phone_type: 2 };
  rows.push(created);
  return created;
}

export function setNational(state: FormState, addressPath: string, national: string, index = 0) {
  const row = addressRow(state, addressPath, index);
  if (!row) return;
  const phones = phoneArray(row, true);
  if (!phones) return;
  const phone = ensurePhone(phones);
  const binding = bindingFor(state, addressPath, index);
  const digits = national.replace(/\D/g, "");
  phone.str_number = binding.callingCode ? binding.callingCode + digits : digits;
  applyMask(state, addressPath, index);
}

function storedNational(state: FormState, addressPath: string, index: number): string {
  const row = addressRow(state, addressPath, index);
  const phones = row ? phoneArray(row, false) : undefined;
  const phone = phones?.[0];
  const stored = phone && typeof phone.str_number === "string" ? phone.str_number : "";
  return nationalDigits(stored, bindingFor(state, addressPath, index).callingCode);
}

function loadedRegionRows(binding: PhoneBinding): EnumRow[] {
  const rows: EnumRow[] = [];
  for (const list of binding.regionMasks.values()) rows.push(...list);
  return rows;
}

function resolveMask(state: FormState, addressPath: string, index: number): string {
  const binding = bindingFor(state, addressPath, index);
  const national = storedNational(state, addressPath, index);
  const regionRows = loadedRegionRows(binding);
  const city = chooseCity(regionRows, national);
  if (city) return city.mask;
  return chooseCountry(binding.countryMasks, national, regionCodes(regionRows));
}

function applyMask(state: FormState, addressPath: string, index: number) {
  const binding = bindingFor(state, addressPath, index);
  binding.mask = resolveMask(state, addressPath, index);
  notifyPhone(state);
}

export function phoneMask(state: FormState, addressPath: string, index = 0): string {
  return bindingFor(state, addressPath, index).mask;
}

export function phoneNational(state: FormState, addressPath: string, index = 0): string {
  return storedNational(state, addressPath, index);
}

export function phoneCountryId(state: FormState, addressPath: string, index = 0): number | null {
  return bindingFor(state, addressPath, index).countryId;
}

const phoneWatchers = new WeakMap<FormState, Set<() => void>>();

function notifyPhone(state: FormState) {
  phoneWatchers.get(state)?.forEach((listener) => listener());
}

export function watchPhone(state: FormState, listener: () => void): () => void {
  const set = phoneWatchers.get(state) ?? new Set();
  set.add(listener);
  phoneWatchers.set(state, set);
  return () => {
    set.delete(listener);
  };
}

function nationalPattern(mask: string, callingCode: string): string {
  if (!mask) return "";
  if (callingCode && mask.startsWith(callingCode)) return mask.slice(callingCode.length).trimStart();
  return mask;
}

export function phoneSlotCount(mask: string, callingCode: string): number {
  return [...nationalPattern(mask, callingCode)].filter((char) => char === "d" || /\d/.test(char)).length;
}

export function phoneText(mask: string, callingCode: string, national: string, focused: boolean): string {
  const pattern = nationalPattern(mask, callingCode);
  if (!pattern) return national;
  let index = 0;
  let out = "";
  for (const char of pattern) {
    if (char === "d" || /\d/.test(char)) {
      if (index >= national.length) out += "_";
      else {
        out += national[index];
        index += 1;
      }
    } else out += char;
  }
  if (focused) return out;
  const cut = out.indexOf("_");
  const head = cut < 0 ? out : out.slice(0, cut);
  return head.replace(/[\s().-]+$/u, "");
}

export type PhoneCountry = { id: number; name: string; code: string };

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

export function matchPhoneCountries(
  rows: PhoneCountry[],
  query: string,
  locale: string,
  currentId: number | null,
): PhoneCountry[] {
  const needle = query.trim().toLocaleLowerCase(locale);
  const codeNeedle = needle.replace(/\D/g, "");
  const matched = needle
    ? rows.filter(
        (row) =>
          startsWord(row.name, needle, locale) ||
          (codeNeedle !== "" && row.code.replace(/\D/g, "").startsWith(codeNeedle)),
      )
    : rows;
  return matched
    .slice()
    .sort((left, right) => {
      if (!needle && currentId !== null) {
        if (left.id === currentId) return -1;
        if (right.id === currentId) return 1;
      }
      return left.name.localeCompare(right.name, locale);
    })
    .slice(0, 12);
}

export async function loadPhoneCountries(
  state: FormState,
  client: Pick<FormClient, "enums">,
): Promise<PhoneCountry[]> {
  const [countries, codes] = await Promise.all([
    client.enums(state.locale, "country", {}),
    client.enums(state.locale, "phone_code", {}),
  ]);
  const codeOf = new Map(codes.list.map((row) => [row.id, text(row.v)]));
  return countries.list.flatMap((row) => {
    const code = codeOf.get(row.id) ?? "";
    const name = text(row.v);
    if (!code || !name) return [];
    return [{ id: row.id, name, code }];
  });
}

export async function openPhone(state: FormState, client: Pick<FormClient, "enums">, questionId: string) {
  const address = phoneAddress(state, questionId);
  if (!address) return;
  const binding = bindingFor(state, address.path, address.index);
  if (binding.countryId !== null || binding.chosen) return;
  const row = addressRow(state, address.path, address.index);
  const country = row?.country_db;
  const countryId = isRecord(country) && typeof country.e_val === "number" && country.e_val >= 0 ? country.e_val : null;
  if (countryId === null) return;
  await applyPhoneCountry(state, client, address.path, address.index, countryId, false);
}

function liveList(state: FormState, path: string): Record<string, unknown>[] {
  const value = readPath(state.visitor, path);
  if (!Array.isArray(value)) return [];
  return value.filter(isRecord);
}

export function matchesPhone(row: Record<string, unknown>, filter: unknown[] | undefined): boolean {
  if (!filter || filter.length === 0) return true;
  return filter.every((clause) => {
    if (!isRecord(clause)) return true;
    return Object.entries(clause).every(([key, expected]) => row[key] === expected);
  });
}

export function shownPhones(state: FormState, path: string, filter?: unknown[]): Record<string, unknown>[] {
  return liveList(state, path).filter((row) => matchesPhone(row, filter));
}

export function addPhone(state: FormState, path: string, filter?: unknown[]): Record<string, unknown> {
  const value = readPath(state.visitor, path);
  if (!Array.isArray(value)) throw new Error(`missing path: ${path}`);
  const row: Record<string, unknown> = { jvlink_id: -1, jvrel_prop: { primary: false }, str_number: "" };
  for (const clause of filter ?? []) {
    if (!isRecord(clause)) continue;
    Object.assign(row, clause);
  }
  value.push(row);
  return row;
}

export function removePhone(state: FormState, path: string, filter: unknown[] | undefined, row: Record<string, unknown>) {
  const value = readPath(state.visitor, path);
  if (!Array.isArray(value)) return;
  const visible = value.filter((item) => isRecord(item) && matchesPhone(item, filter));
  if (visible.length <= 1) return;
  const index = value.indexOf(row);
  if (index >= 0) value.splice(index, 1);
}

export function setPhonePrimary(state: FormState, path: string, filter: unknown[] | undefined, row: Record<string, unknown>) {
  for (const item of liveList(state, path)) {
    if (!matchesPhone(item, filter)) continue;
    const prop = isRecord(item.jvrel_prop) ? item.jvrel_prop : {};
    item.jvrel_prop = prop;
    prop.primary = item === row;
  }
}

onAddressCountry((state, client, path, index, countryId) => {
  const binding = bindingFor(state, path, index);
  if (binding.chosen || storedNational(state, path, index) !== "") return;
  void schedule(state, () => applyPhoneCountry(state, client, path, index, countryId, false)).catch(() => undefined);
});
