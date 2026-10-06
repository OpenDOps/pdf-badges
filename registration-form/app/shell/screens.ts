import type { ComponentType } from "react";
import type { LoaderFunction } from "react-router";

type ScreenModule = {
  default: ComponentType;
  loader?: LoaderFunction;
};

type ScreenFile =
  | "routes/key.tsx"
  | "routes/menu.tsx"
  | "routes/desk.tsx"
  | "routes/visitor.tsx"
  | "routes/visitors.tsx"
  | "routes/print.tsx"
  | "routes/import.tsx"
  | "routes/settings.tsx"
  | "routes/printers.tsx"
  | "routes/registration-settings.tsx";

const screens: Record<string, { file: ScreenFile; load: () => Promise<ScreenModule> }> = {
  "/key": { file: "routes/key.tsx", load: () => import("../routes/key") },
  "/": { file: "routes/menu.tsx", load: () => import("../routes/menu") },
  "/form": { file: "routes/desk.tsx", load: () => import("../routes/desk") },
  "/visitor": { file: "routes/visitor.tsx", load: () => import("../routes/visitor") },
  "/visitors": { file: "routes/visitors.tsx", load: () => import("../routes/visitors") },
  "/print": { file: "routes/print.tsx", load: () => import("../routes/print") },
  "/import": { file: "routes/import.tsx", load: () => import("../routes/import") },
  "/settings": { file: "routes/settings.tsx", load: () => import("../routes/settings") },
  "/printers": { file: "routes/printers.tsx", load: () => import("../routes/printers") },
  "/settings/registration": {
    file: "routes/registration-settings.tsx",
    load: () => import("../routes/registration-settings"),
  },
};

const loads: ScreenFile[] = [];

export function resetScreenLoads() {
  loads.length = 0;
}

export function screenLoads(): readonly ScreenFile[] {
  return loads;
}

export async function loadScreen(path: string) {
  const screen = screens[path.split(/[?#]/)[0]];
  if (!screen) throw new Error(`unknown screen ${path}`);
  loads.push(screen.file);
  const module = await screen.load();
  return module.loader ? { Component: module.default, loader: module.loader } : { Component: module.default };
}
