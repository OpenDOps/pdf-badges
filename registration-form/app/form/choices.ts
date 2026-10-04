import type { FormState, Question } from "./engine";

export type ChoiceOption = {
  id: number;
  label: string;
  depth: number;
};

type ChoiceNode = {
  id: number;
  labels: { ru: string; en: string };
  depth: number;
  children: ChoiceNode[];
};

const lists = new WeakMap<Question, ChoiceNode[]>();

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function labelsOf(value: unknown): { ru: string; en: string } {
  if (typeof value === "string") return { ru: value, en: value };
  if (!isRecord(value)) return { ru: "", en: "" };
  const ru = typeof value.ru === "string" ? value.ru : "";
  const en = typeof value.en === "string" ? value.en : "";
  return { ru, en };
}

function nodesOf(value: unknown, depth: number): ChoiceNode[] {
  if (!Array.isArray(value)) return [];
  const nodes: ChoiceNode[] = [];
  for (const item of value) {
    if (!isRecord(item) || typeof item.id !== "number") continue;
    nodes.push({
      id: item.id,
      labels: labelsOf(item.v),
      depth,
      children: nodesOf(item.opts, depth + 1),
    });
  }
  return nodes;
}

export function bindChoices(question: Question, list: unknown) {
  lists.set(question, nodesOf(list, 0));
}

function nodesFor(question: Question): ChoiceNode[] {
  return lists.get(question) ?? [];
}

function idsOf(nodes: ChoiceNode[], ids: Set<number>) {
  for (const node of nodes) {
    ids.add(node.id);
    idsOf(node.children, ids);
  }
}

export function knownChoice(question: Question, value: unknown): boolean {
  if (value === null || value === undefined) return true;
  const ids = Array.isArray(value) ? value : [value];
  if (ids.length === 0) return true;
  const known = new Set<number>();
  idsOf(nodesFor(question), known);
  return ids.every((id) => typeof id === "number" && (known.has(id) || question.other));
}

function sorted(nodes: ChoiceNode[], locale: string): ChoiceOption[] {
  const ordered = nodes.slice().sort((left, right) => left.id - right.id);
  const options: ChoiceOption[] = [];
  for (const node of ordered) {
    const label = locale === "en" ? node.labels.en : node.labels.ru;
    options.push({ id: node.id, label, depth: node.depth });
    options.push(...sorted(node.children, locale));
  }
  return options;
}

export function choiceOptions(state: FormState, id: string, locale = state.locale): ChoiceOption[] {
  const question = state.questions[id];
  if (!question) return [];
  return sorted(nodesFor(question), locale);
}

function parentAt(root: Record<string, unknown>, path: string): { parent: Record<string, unknown>; key: string } {
  const parts = path.split(".");
  let current: unknown = root;
  for (let index = 0; index < parts.length - 1; index += 1) {
    const node = Array.isArray(current) ? current[0] : current;
    if (!isRecord(node)) throw new Error(`missing path: ${path}`);
    current = node[parts[index]];
  }
  const parent = Array.isArray(current) ? current[0] : current;
  if (!isRecord(parent)) throw new Error(`missing path: ${path}`);
  return { parent, key: parts[parts.length - 1] };
}

function stored(state: FormState, id: string): unknown {
  const question = state.questions[id];
  if (!question?.leaf || question.leaf.kind !== "option") return undefined;
  const { parent, key } = parentAt(state.visitor, id);
  const node = parent[key];
  return isRecord(node) ? node.o_val : undefined;
}

function writeOption(state: FormState, id: string, value: unknown) {
  const question = state.questions[id];
  if (!question?.leaf || question.leaf.kind !== "option") throw new Error(`missing path: ${id}`);
  if (!knownChoice(question, value)) throw new Error("unknown choice");
  const { parent, key } = parentAt(state.visitor, id);
  const node = parent[key];
  if (!isRecord(node)) throw new Error(`missing path: ${id}`);
  node.o_val = value;
}

function empty(value: unknown): boolean {
  if (value === null || value === undefined) return true;
  if (typeof value === "string") return value.trim() === "";
  if (Array.isArray(value)) return value.length === 0;
  return false;
}

export function applyChoiceDefaults(state: FormState) {
  for (const question of Object.values(state.questions)) {
    if (question.leaf?.kind !== "option" || question.def === undefined) continue;
    if (!empty(stored(state, question.id))) continue;
    writeOption(state, question.id, question.many ? [question.def] : question.def);
  }
}

export function choose(state: FormState, id: string, optionId: number) {
  const question = state.questions[id];
  if (!question) throw new Error(`missing path: ${id}`);
  if (!question.many) {
    writeOption(state, id, optionId);
    return;
  }
  const current = stored(state, id);
  const list = Array.isArray(current) ? current.filter((item): item is number => typeof item === "number") : [];
  const index = list.indexOf(optionId);
  if (index >= 0) list.splice(index, 1);
  else list.push(optionId);
  writeOption(state, id, list);
}

function writeText(state: FormState, id: string, text: string) {
  const question = state.questions[id];
  if (!question?.leaf) throw new Error(`missing path: ${id}`);
  const { parent, key } = parentAt(state.visitor, id);
  if (question.leaf.kind === "parametric") {
    const current = parent[key];
    const node: Record<string, unknown> = isRecord(current) ? current : {};
    if (current !== node) parent[key] = node;
    const localeNode = node[state.locale];
    if (isRecord(localeNode)) localeNode.str = text;
    else node[state.locale] = { str: text };
    return;
  }
  parent[key] = text;
}

export function setOtherLine(state: FormState, id: string, text: string) {
  const question = state.questions[id];
  if (!question) throw new Error(`missing path: ${id}`);
  question.otherText = text;
  if (question.bindedOther) writeText(state, question.bindedOther, text);
}
