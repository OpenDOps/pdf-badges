import type { FormState } from "./engine";

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

function liveList(state: FormState, path: string): unknown[] {
  const value = readPath(state.visitor, path);
  if (!Array.isArray(value)) throw new Error(`missing path: ${path}`);
  return value;
}

export function addEmail(state: FormState, path: string): Record<string, unknown> {
  const rows = liveList(state, path);
  const row = { jvlink_id: -1, jvrel_prop: { primary: false }, email: "" };
  rows.push(row);
  return row;
}

export function removeEmail(state: FormState, path: string, row: Record<string, unknown>) {
  const rows = liveList(state, path);
  const records = rows.filter(isRecord);
  if (records.length <= 1) return;
  const index = rows.indexOf(row);
  if (index >= 0) rows.splice(index, 1);
}

export function setEmailPrimary(state: FormState, path: string, row: Record<string, unknown>) {
  for (const item of liveList(state, path)) {
    if (!isRecord(item)) continue;
    const prop = isRecord(item.jvrel_prop) ? item.jvrel_prop : {};
    item.jvrel_prop = prop;
    prop.primary = item === row;
  }
}
