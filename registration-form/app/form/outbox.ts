export const OUTBOX_KEY = "registration-form.outbox";

export function readOutbox(): unknown[] {
  try {
    const raw = localStorage.getItem(OUTBOX_KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : [];
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
}

export function writeOutbox(items: unknown[]) {
  localStorage.setItem(OUTBOX_KEY, JSON.stringify(items));
}

export function enqueueVisitor(visitor: unknown) {
  const items = readOutbox();
  items.push(structuredClone(visitor));
  writeOutbox(items);
}
