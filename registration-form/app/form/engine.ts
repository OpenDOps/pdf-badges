import type { EnumRow, FormDocuments, Screen } from "./client";
import { applyChoiceDefaults, bindChoices, knownChoice } from "./choices";
import { collectLeaves, type Leaf } from "./leaves";
import { phoneHasNationalDigits } from "./phones";

export type Question = {
  id: string;
  hidden: boolean;
  leaf: Leaf | null;
  many: boolean;
  radios: boolean;
  req: boolean;
  reqLoc: Record<string, boolean>;
  reqIndex?: number;
  otherReq: boolean;
  other: boolean;
  otherId?: number;
  otherText: string;
  bindedOther?: string;
  def?: number;
  uniqueNum?: number;
  dontchange: boolean;
  disabled: boolean;
  warned: boolean;
  label: { ru?: string; en?: string };
};

export type PlaceList = {
  list: EnumRow[];
  error: Error | null;
  def?: number;
  ui: boolean;
};

export type PlaceBinding = {
  path: string;
  index: number;
  country?: PlaceList;
  region?: PlaceList;
  city?: PlaceList;
};

export type StepError = {
  id: string;
  reason: "required" | "other" | "unique" | "email";
};

export type FormState = {
  locale: string;
  visitor: Record<string, unknown>;
  questions: Record<string, Question>;
  rows: string[][];
  screens: Screen[];
  step: number[];
  past: number[][];
  grouped: boolean;
  ended: boolean;
  errors: StepError[];
  places: Record<string, PlaceBinding>;
  countryFields: Record<string, true>;
  countryFocus?: string;
  searched: boolean;
};

type QuestionSource = {
  id: string;
  chboxes?: boolean;
  single_select?: boolean;
  req?: boolean;
  req_loc?: Record<string, boolean>;
  req_index?: number;
  other?: boolean;
  other_req?: boolean;
  binded_other?: string;
  select?: boolean;
  tree?: boolean;
  dbenum_def?: number;
  dbenum_defs?: number[];
  unique_num?: number;
  dontchange?: boolean;
  disabled?: boolean;
  ru?: string;
  en?: string;
};

type Rule = {
  step: number;
  payment?: boolean;
  conditions?: {
    operator?: string;
    expression?: { q_id: string; value: unknown }[];
  };
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function normalizeEnums(value: unknown) {
  if (Array.isArray(value)) {
    for (const item of value) normalizeEnums(item);
    return;
  }
  if (!isRecord(value)) return;
  if (value.e_val === -1) value.e_val = null;
  for (const child of Object.values(value)) normalizeEnums(child);
}

function primary(rows: unknown[]): Record<string, unknown> {
  const records = rows.filter(isRecord);
  return (
    records.find((row) => isRecord(row.jvrel_prop) && row.jvrel_prop.primary === true) ??
    records[0]
  );
}

function parentAt(root: Record<string, unknown>, path: string): { parent: Record<string, unknown>; key: string } {
  const parts = path.split(".");
  let current: unknown = root;
  for (let index = 0; index < parts.length - 1; index += 1) {
    const node = Array.isArray(current) ? primary(current) : current;
    if (!isRecord(node)) throw new Error(`missing path: ${path}`);
    current = node[parts[index]];
  }
  const parent = Array.isArray(current) ? primary(current) : current;
  if (!isRecord(parent)) throw new Error(`missing path: ${path}`);
  return { parent, key: parts[parts.length - 1] };
}

function otherId(vars: Record<string, unknown>, leaf: Leaf | null, other: boolean): number | undefined {
  if (!other || !leaf?.optionKey) return undefined;
  const list = vars[leaf.optionKey];
  if (!Array.isArray(list) || list.length === 0) return undefined;
  const last = list[list.length - 1];
  if (!isRecord(last) || typeof last.id !== "number") return undefined;
  return last.id;
}

function bind(source: QuestionSource, leaves: Map<string, Leaf>, vars: Record<string, unknown>, hidden: boolean): Question {
  const leaf = leaves.get(source.id) ?? null;
  const single = source.single_select === true || leaf?.single === true;
  const listed = (source.chboxes === true || source.tree === true) && source.select !== true;
  const question: Question = {
    id: source.id,
    hidden,
    leaf,
    many: listed && !single,
    radios: listed && single,
    req: source.req === true,
    reqLoc: source.req_loc ?? {},
    reqIndex: source.req_index,
    otherReq: source.other === true && source.other_req === true,
    other: source.other === true,
    otherId: otherId(vars, leaf, source.other === true),
    otherText: "",
    bindedOther: typeof source.binded_other === "string" ? source.binded_other : undefined,
    def: questionDefault(source),
    uniqueNum: typeof source.unique_num === "number" ? source.unique_num : undefined,
    dontchange: source.dontchange === true,
    disabled: source.disabled === true,
    warned: false,
    label: {
      ru: typeof source.ru === "string" ? source.ru : undefined,
      en: typeof source.en === "string" ? source.en : undefined,
    },
  };
  if (leaf?.kind === "option") bindChoices(question, leaf.optionKey ? vars[leaf.optionKey] : undefined);
  return question;
}

function questionDefault(source: QuestionSource): number | undefined {
  if (typeof source.dbenum_def === "number") return source.dbenum_def;
  if (Array.isArray(source.dbenum_defs) && typeof source.dbenum_defs[0] === "number") return source.dbenum_defs[0];
  return undefined;
}

function screenMatches(screen: Screen, locale: string): boolean {
  return !screen.for_locales || screen.for_locales.length === 0 || screen.for_locales.includes(locale);
}

function blockStart(screens: Screen[], index: number): number {
  const block = screens[index]?.screen_block;
  if (block === undefined || block === null) return index;
  let start = index;
  while (start > 0 && screens[start - 1]?.screen_block === block) start -= 1;
  return start;
}

function stepAt(screens: Screen[], start: number, grouped: boolean): number[] {
  const indexes = [start];
  if (!grouped) return indexes;
  const block = screens[start]?.screen_block;
  if (block === undefined || block === null) return indexes;
  for (let index = start + 1; index < screens.length; index += 1) {
    if (screens[index]?.screen_block === block) indexes.push(index);
  }
  return indexes;
}

function firstStep(screens: Screen[], locale: string, grouped: boolean): number[] {
  const start = screens.findIndex((screen) => screenMatches(screen, locale));
  if (start < 0) return [];
  return stepAt(screens, start, grouped);
}

export function setGrouped(state: FormState, grouped: boolean) {
  if (state.grouped === grouped) return;
  state.grouped = grouped;
  const current = state.step[0];
  if (current === undefined) return;
  state.step = grouped ? stepAt(state.screens, blockStart(state.screens, current), true) : [current];
}

function questionIds(state: FormState, step: number[]): string[] {
  const ids: string[] = [];
  for (const index of step) {
    const screen = state.screens[index];
    if (!screen) continue;
    for (const item of screen.body) {
      const group = Array.isArray(item) ? item : [item];
      for (const source of group) {
        if (isRecord(source) && typeof source.id === "string") ids.push(source.id);
      }
    }
  }
  return ids;
}

export function readValue(state: FormState, id: string, locale: string): unknown {
  const question = state.questions[id];
  if (!question?.leaf) return undefined;
  const { parent, key } = parentAt(state.visitor, id);
  const node = parent[key];
  if (question.leaf.kind === "enum") return isRecord(node) ? node.e_val : undefined;
  if (question.leaf.kind === "option") return isRecord(node) ? node.o_val : undefined;
  if (question.leaf.kind === "parametric") {
    if (!isRecord(node)) return "";
    const localeNode = node[locale];
    return isRecord(localeNode) ? localeNode.str : "";
  }
  return node;
}

function hasValue(value: unknown): boolean {
  if (value === null || value === undefined) return false;
  if (typeof value === "string") return value.trim().length > 0;
  if (Array.isArray(value)) return value.length > 0;
  return true;
}

function isPhoneNumber(id: string): boolean {
  return /(?:contact_phones|phones_faxes|faxes)\.str_number$/.test(id);
}

function isEmail(id: string): boolean {
  return id.endsWith("emails.email");
}

export function emailShape(value: string): boolean {
  const trimmed = value.trim();
  const at = trimmed.indexOf("@");
  if (at <= 0 || trimmed.includes("@", at + 1)) return false;
  const labels = trimmed.slice(at + 1).split(".");
  if (labels.length < 2 || labels.some((label) => label.length === 0)) return false;
  const zone = labels[labels.length - 1] ?? "";
  return /^\p{L}{2,}$/u.test(zone);
}

function filledValue(state: FormState, id: string, locale: string): boolean {
  const value = readValue(state, id, locale);
  if (isPhoneNumber(id)) return phoneHasNationalDigits(state, id, value);
  return hasValue(value);
}

function requiredNow(question: Question, locale: string): boolean {
  return question.req || question.reqLoc[locale] === true;
}

function clauseMatches(state: FormState, clause: { q_id: string; value: unknown }): boolean {
  const value = readValue(state, clause.q_id, state.locale);
  if (Array.isArray(value)) return value.includes(clause.value);
  return value === clause.value;
}

function rulePasses(state: FormState, rule: Rule): boolean {
  const expression = rule.conditions?.expression;
  if (!rule.conditions || !expression) return true;
  const matches = expression.map((clause) => clauseMatches(state, clause));
  if (rule.conditions.operator === "or") return matches.some(Boolean);
  return matches.every(Boolean);
}

function isRule(value: unknown): value is Rule {
  return isRecord(value) && typeof value.step === "number";
}

export function openScreens(state: FormState): number[] {
  const shown: number[] = [];
  const seen = new Set<number>();
  const start = state.screens.findIndex((screen) => screenMatches(screen, state.locale));
  if (start < 0) return shown;
  const queue = [start];
  while (queue.length > 0) {
    const index = queue.shift();
    if (index === undefined || seen.has(index) || index < 0 || index >= state.screens.length) continue;
    seen.add(index);
    const screen = state.screens[index];
    if (!screen) continue;
    if (screenMatches(screen, state.locale)) shown.push(index);
    for (const rule of (screen.next ?? []).filter(isRule)) {
      if (!rulePasses(state, rule)) continue;
      const nextScreen = rule.step >= 0 ? state.screens[rule.step] : undefined;
      if (rule.step < 0 || rule.payment === true || nextScreen?.payment === true) continue;
      if (!seen.has(rule.step)) queue.push(rule.step);
    }
  }
  return shown;
}

export function build(documents: FormDocuments, locale: string): FormState {
  const visitor = structuredClone(documents.model);
  normalizeEnums(visitor);
  const leaves = collectLeaves(documents.struct, documents.vars);
  const questions: Record<string, Question> = {};
  const rows: string[][] = [];

  for (const screen of documents.conf) {
    for (const item of screen.body) {
      const group = Array.isArray(item) ? item : [item];
      const ids: string[] = [];
      for (const source of group) {
        if (!isRecord(source) || typeof source.id !== "string") continue;
        ids.push(source.id);
        if (!questions[source.id]) questions[source.id] = bind(source as QuestionSource, leaves, documents.vars, false);
      }
      if (ids.length > 0) rows.push(ids);
    }
  }

  for (const source of documents.hiddenq) {
    if (!isRecord(source) || typeof source.id !== "string") continue;
    if (!questions[source.id]) questions[source.id] = bind(source as QuestionSource, leaves, documents.vars, true);
  }

  const state: FormState = {
    locale,
    visitor,
    questions,
    rows,
    screens: documents.conf,
    step: firstStep(documents.conf, locale, true),
    past: [],
    grouped: true,
    ended: false,
    errors: [],
    places: {},
    countryFields: {},
    searched: false,
  };
  applyChoiceDefaults(state);
  return state;
}

export const EMAIL_CHANGE_WARNING =
  "This changes the person already registered on that email. It does not start a new person.";

export const SEARCHED_CHANGE_WARNING =
  "This changes the person already loaded from search. It does not start a new person.";

export function markSearched(state: FormState) {
  state.searched = true;
}

export function loadVisitor(state: FormState, visitor: Record<string, unknown>) {
  const next = structuredClone(visitor);
  normalizeEnums(next);
  state.visitor = next;
  state.searched = true;
  for (const question of Object.values(state.questions)) {
    question.warned = false;
    question.otherText = "";
  }
  state.past = [];
  state.errors = [];
  state.ended = false;
  state.places = {};
  state.countryFields = {};
  state.countryFocus = undefined;
  state.step = firstStep(state.screens, state.locale, state.grouped);
}

export function setValue(state: FormState, id: string, locale: string, value: unknown): string | undefined {
  const question = state.questions[id];
  if (!question?.leaf) throw new Error(`missing path: ${id}`);
  if (question.disabled) return;
  if (question.dontchange && state.searched && !question.warned) {
    question.warned = true;
    return isEmail(id) ? EMAIL_CHANGE_WARNING : SEARCHED_CHANGE_WARNING;
  }
  const { parent, key } = parentAt(state.visitor, id);
  const leaf = question.leaf;

  if (leaf.kind === "enum") {
    if (leaf.enumName === "region") throw new Error("region is not set from the UI");
    const node = parent[key];
    if (!isRecord(node)) throw new Error(`missing path: ${id}`);
    node.e_val = value;
    return;
  }
  if (leaf.kind === "option") {
    const node = parent[key];
    if (!isRecord(node)) throw new Error(`missing path: ${id}`);
    if (!knownChoice(question, value)) throw new Error("unknown choice");
    node.o_val = value;
    return;
  }
  if (leaf.kind === "parametric") {
    const current = parent[key];
    const node: Record<string, unknown> = isRecord(current)
      ? current
      : { ru: { str: typeof current === "string" ? current : "" }, en: { str: typeof current === "string" ? current : "" } };
    if (current !== node) parent[key] = node;
    const localeNode = node[locale];
    if (isRecord(localeNode)) localeNode.str = value;
    else node[locale] = { str: value };
    return;
  }
  parent[key] = value;
}

export function check(state: FormState, step: number[], locale: string): StepError[] {
  const ids = questionIds(state, step);
  const groups = new Map<number, string[]>();
  for (const id of ids) {
    const question = state.questions[id];
    if (!question || question.reqIndex === undefined || !requiredNow(question, locale)) continue;
    const members = groups.get(question.reqIndex) ?? [];
    members.push(id);
    groups.set(question.reqIndex, members);
  }

  const errors: StepError[] = [];
  const reported = new Set<number>();
  for (const id of ids) {
    const question = state.questions[id];
    if (!question) continue;
    if (requiredNow(question, locale)) {
      if (question.reqIndex !== undefined) {
        if (!reported.has(question.reqIndex)) {
          reported.add(question.reqIndex);
          const members = groups.get(question.reqIndex) ?? [id];
          if (!members.some((member) => filledValue(state, member, locale))) {
            errors.push({ id, reason: "required" });
          }
        }
      } else if (!filledValue(state, id, locale)) {
        errors.push({ id, reason: "required" });
      }
    } else if (isPhoneNumber(id) && !phoneHasNationalDigits(state, id, readValue(state, id, locale)) && hasValue(readValue(state, id, locale))) {
      errors.push({ id, reason: "required" });
    }
    if (question.otherReq && question.otherId !== undefined) {
      const value = readValue(state, id, locale);
      const selected = Array.isArray(value) ? value.includes(question.otherId) : value === question.otherId;
      if (selected && question.otherText.trim().length === 0) errors.push({ id, reason: "other" });
    }
    if (isEmail(id)) {
      const value = readValue(state, id, locale);
      if (typeof value === "string" && value.trim() !== "" && !emailShape(value)) errors.push({ id, reason: "email" });
    }
  }
  return errors;
}

function follow(state: FormState) {
  const last = state.step[state.step.length - 1];
  const screen = last === undefined ? undefined : state.screens[last];
  const rules = (screen?.next ?? []).filter(isRule);
  for (const rule of rules) {
    if (!rulePasses(state, rule)) continue;
    const destination = rule.step >= 0 ? state.screens[rule.step] : undefined;
    if (rule.step === -1 || rule.payment === true || destination?.payment === true) {
      state.ended = true;
      state.errors = [];
      return;
    }
    state.past.push(state.step);
    state.step = stepAt(state.screens, rule.step, state.grouped);
    state.ended = false;
    state.errors = [];
    return;
  }
}

export async function checkStep(
  state: FormState,
  client: { counts(kind: "email" | "phone", value: string): Promise<{ count: number }> },
): Promise<StepError[]> {
  const errors = check(state, state.step, state.locale);
  for (const id of questionIds(state, state.step)) {
    const question = state.questions[id];
    if (!question?.uniqueNum || question.uniqueNum <= 0) continue;
    const value = readValue(state, id, state.locale);
    if (typeof value !== "string") continue;
    try {
      if (isPhoneNumber(id)) {
        if (!phoneHasNationalDigits(state, id, value)) continue;
        const result = await client.counts("phone", value);
        if (result.count >= question.uniqueNum) errors.push({ id, reason: "unique" });
      }
      if (isEmail(id)) {
        if (!emailShape(value)) continue;
        const result = await client.counts("email", value);
        if (result.count >= question.uniqueNum) errors.push({ id, reason: "unique" });
      }
    } catch {
      // A missing count service leaves the field's own checks in place.
    }
  }
  return errors;
}

export async function next(
  state: FormState,
  client: { counts(kind: "email" | "phone", value: string): Promise<{ count: number }> },
): Promise<StepError[]> {
  const errors = await checkStep(state, client);
  state.errors = errors;
  if (errors.length > 0) return errors;
  follow(state);
  return [];
}

export function back(state: FormState): boolean {
  const previous = state.past.pop();
  if (!previous) return false;
  state.step = previous;
  state.ended = false;
  state.errors = [];
  return true;
}

export function setLocale(state: FormState, locale: string) {
  state.locale = locale;
  const included = state.step.some((index) => {
    const screen = state.screens[index];
    return screen ? screenMatches(screen, locale) : false;
  });
  if (included || state.step.length === 0) return;
  const after = Math.max(...state.step);
  for (let index = after + 1; index < state.screens.length; index += 1) {
    const screen = state.screens[index];
    if (screen && screenMatches(screen, locale)) {
      state.step = stepAt(state.screens, index, state.grouped);
      state.ended = false;
      return;
    }
  }
}
