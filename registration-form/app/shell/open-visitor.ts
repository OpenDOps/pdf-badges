import { useSyncExternalStore } from "react";
import type { NavigateFunction } from "react-router";

export type OpenVisitor = { uid: string };

let current: OpenVisitor | null = null;
let dirty = false;
const listeners = new Set<() => void>();
const dirtyListeners = new Set<() => void>();

function emit() {
  listeners.forEach((listener) => listener());
}

function emitDirty() {
  dirtyListeners.forEach((listener) => listener());
}

export function visitorHref(uid: string) {
  return `/visitor?userId=${encodeURIComponent(uid)}`;
}

export function openVisitor(uid: string) {
  current = { uid };
  emit();
}

export function closeVisitor() {
  current = null;
  dirty = false;
  emit();
  emitDirty();
}

/** Leave `/visitor` by popping history (the list that opened it), not pushing `/visitors`. */
export function leaveOpenedVisitor(navigate: NavigateFunction) {
  const idx = (window.history.state as { idx?: number } | null)?.idx;
  if (typeof idx === "number" && idx > 0) {
    navigate(-1);
    return;
  }
  navigate("/visitors", { replace: true });
}

export function resetOpenVisitor() {
  current = null;
  dirty = false;
  emit();
  emitDirty();
}

export function setVisitorDirty(value: boolean) {
  if (dirty === value) return;
  dirty = value;
  emitDirty();
}

export function useVisitorDirty(): boolean {
  return useSyncExternalStore(
    (listener) => {
      dirtyListeners.add(listener);
      return () => dirtyListeners.delete(listener);
    },
    () => dirty,
    () => false,
  );
}

export function useOpenVisitor(): OpenVisitor | null {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    () => current,
    () => null,
  );
}
