import { readFileSync } from "node:fs";
import path from "node:path";

import type { FormDocuments, Screen } from "./client";

function stripRights(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(stripRights);
  if (value && typeof value === "object") {
    const out: Record<string, unknown> = {};
    for (const [key, child] of Object.entries(value)) {
      if (key === "jv_rights" || key.endsWith("_jv_rights")) continue;
      out[key] = stripRights(child);
    }
    return out;
  }
  return value;
}

function readJson(dir: string, name: string): unknown {
  return JSON.parse(readFileSync(path.join(dir, name), "utf8")) as unknown;
}

export function formsDirectory(): string {
  return path.join(process.cwd(), "public", "forms");
}

export function foldForm(formsDir: string = formsDirectory()): FormDocuments {
  const questions = readJson(formsDir, "quest_conf.json") as {
    conf: Screen[];
    hiddenq: unknown[];
  };
  return {
    vars: readJson(formsDir, "user_vars.json") as Record<string, unknown>,
    struct: readJson(formsDir, "user_struct.json"),
    model: stripRights(readJson(formsDir, "user_model.json")) as FormDocuments["model"],
    conf: questions.conf,
    hiddenq: questions.hiddenq,
    settings: readJson(formsDir, "glob_stngs.json"),
    barcodes: readJson(formsDir, "totalbarcodes.json"),
  };
}
