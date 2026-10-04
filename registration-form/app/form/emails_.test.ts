import { readFile } from "node:fs/promises";
import path from "node:path";

import type { FormDocuments } from "./client";
import { FormClient } from "./client";
import { EMAIL_CHANGE_WARNING, SEARCHED_CHANGE_WARNING, build, check, markSearched, next, setValue } from "./engine";
import { foldForm } from "./fold";

const PERSONAL = "personalData.emails.email";
const COMPANY = "personalData.companies.addresses.emails.email";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function personalEmail(state: ReturnType<typeof build>): Record<string, unknown> {
  const personal = state.visitor.personalData;
  if (!isRecord(personal) || !Array.isArray(personal.emails)) throw new Error("emails missing");
  const row = personal.emails[0];
  if (!isRecord(row)) throw new Error("email missing");
  return row;
}

function questions(documents: FormDocuments): Record<string, unknown>[] {
  const list: Record<string, unknown>[] = [];
  for (const screen of documents.conf) {
    for (const item of screen.body) {
      const group = Array.isArray(item) ? item : [item];
      for (const source of group) {
        if (isRecord(source)) list.push(source);
      }
    }
  }
  return list;
}

function recordAt(root: unknown, parts: string[]): Record<string, unknown> {
  let current = root;
  for (const part of parts) {
    if (!isRecord(current)) throw new Error(`${part} missing`);
    current = current[part];
  }
  if (!isRecord(current)) throw new Error("record missing");
  return current;
}

function injectCompanyEmail(documents: FormDocuments) {
  const addressLinks = recordAt(documents.struct, [
    "personalData",
    "possible_links",
    "companies",
    "possible_links",
    "addresses",
    "possible_links",
  ]);
  addressLinks.emails = {
    rel_prop_unique: ["primary"],
    possible_links: { email: "", jvrel_prop: { primary: "boolean" } },
  };
  const personal = documents.model.personalData;
  if (!isRecord(personal) || !Array.isArray(personal.companies)) throw new Error("companies missing");
  const company = personal.companies[0];
  if (!isRecord(company) || !Array.isArray(company.addresses)) throw new Error("addresses missing");
  const row = company.addresses[0];
  if (!isRecord(row)) throw new Error("address missing");
  row.emails = [{ jvlink_id: -1, jvrel_prop: { primary: true }, email: "" }];
  documents.conf[0]?.body.push({ id: COMPANY, req: true, req_index: 1 });
}

function publicFetch() {
  const urls: string[] = [];
  const impl: typeof fetch = async (input) => {
    const raw = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    const pathname = raw.startsWith("/") ? raw : new URL(raw).pathname;
    urls.push(pathname);
    if (pathname.startsWith("/api/registrations/counts")) {
      return new Response(JSON.stringify({ count: 1 }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      });
    }
    const file = path.join(process.cwd(), "public", ...pathname.split("/").filter(Boolean));
    const body = await readFile(file);
    return new Response(body, { status: 200, headers: { "Content-Type": "application/json" } });
  };
  return { impl, urls };
}

test("emails_needs_a_domain_dot", () => {
  const state = build(foldForm(), "ru");
  for (const value of ["a@b", "kuprin.al@gm.", "a@b.c"]) {
    personalEmail(state).email = value;
    expect(check(state, state.step, "ru").some((error) => error.id === PERSONAL && error.reason === "email")).toBe(true);
  }
  personalEmail(state).email = "a@b.co";
  expect(check(state, state.step, "ru").some((error) => error.id === PERSONAL && error.reason === "email")).toBe(false);
});

test("emails_unique_num", async () => {
  const transport = publicFetch();
  const client = new FormClient(transport.impl);
  const state = build(foldForm(), "ru");
  personalEmail(state).email = "a@b.co";
  const step = state.step.slice();
  const errors = await next(state, client);
  expect(errors).toContainEqual({ id: PERSONAL, reason: "unique" });
  expect(state.step).toEqual(step);
  expect(state.ended).toBe(false);
  expect(transport.urls.some((url) => decodeURIComponent(url).includes("a@b.co"))).toBe(true);
});

test("emails_dontchange_warns_once", () => {
  const state = build(foldForm(), "ru");
  const question = state.questions[PERSONAL];
  if (!question) throw new Error("email missing");
  question.dontchange = true;
  markSearched(state);
  personalEmail(state).email = "old@a.b";
  expect(setValue(state, PERSONAL, "ru", "new@a.b")).toBe(EMAIL_CHANGE_WARNING);
  expect(personalEmail(state).email).toBe("old@a.b");
  expect(setValue(state, PERSONAL, "ru", "new@a.b")).toBeUndefined();
  expect(personalEmail(state).email).toBe("new@a.b");
});

test("emails_dontchange_surname_has_its_own_warning", () => {
  const state = build(foldForm(), "ru");
  const id = "personalData.surname";
  const question = state.questions[id];
  if (!question) throw new Error("surname missing");
  question.dontchange = true;
  markSearched(state);
  const personal = state.visitor.personalData;
  if (!isRecord(personal)) throw new Error("surname missing");
  expect(personal.surname).toBe("");
  expect(setValue(state, id, "ru", "Новая")).toBe(SEARCHED_CHANGE_WARNING);
  expect(personal.surname).toBe("");
  expect(setValue(state, id, "ru", "Новая")).toBeUndefined();
  expect(personal.surname).toBe("Новая");
});

test("emails_disabled_keeps_the_value", () => {
  const state = build(foldForm(), "ru");
  const question = state.questions[PERSONAL];
  if (!question) throw new Error("email missing");
  question.disabled = true;
  personalEmail(state).email = "old@a.b";
  setValue(state, PERSONAL, "ru", "new@a.b");
  expect(personalEmail(state).email).toBe("old@a.b");
});

test("emails_one_of_the_pair_is_enough", () => {
  const documents = foldForm();
  injectCompanyEmail(documents);
  const personal = questions(documents).find((question) => question.id === PERSONAL);
  if (!personal) throw new Error("personal email missing");
  personal.req = true;
  personal.req_index = 1;
  for (const question of questions(documents)) {
    if (question.id === PERSONAL || question.id === COMPANY) continue;
    question.req = false;
  }
  const state = build(documents, "ru");
  setValue(state, PERSONAL, "ru", "a@b.co");
  expect(check(state, state.step, "ru")).toEqual([]);
});
