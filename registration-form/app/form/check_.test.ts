import type { FormDocuments } from "./client";
import { build, check, next, setValue } from "./engine";
import { foldForm } from "./fold";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
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

test("check_required_empty", async () => {
  const state = build(foldForm(), "ru");
  const errors = check(state, state.step, "ru");
  expect(errors[0]).toEqual({ id: "personalData.surname", reason: "required" });
  const step = [...state.step];
  await next(state, { counts: async () => ({ count: 0 }) });
  expect(state.step).toEqual(step);
  expect(state.errors[0]?.id).toBe("personalData.surname");
});

test("check_required_for_one_locale", () => {
  const documents = foldForm();
  for (const question of questions(documents)) question.req = false;
  const surname = questions(documents).find((question) => question.id === "personalData.surname");
  if (!surname) throw new Error("surname missing");
  surname.req_loc = { ru: true };
  const state = build(documents, "ru");
  expect(check(state, state.step, "ru")[0]).toEqual({ id: "personalData.surname", reason: "required" });
  expect(check(state, state.step, "en")).toEqual([]);
});

test("check_req_index_group", () => {
  const documents = foldForm();
  for (const question of questions(documents)) question.req = false;
  for (const id of ["personalData.surname", "personalData.name"]) {
    const question = questions(documents).find((item) => item.id === id);
    if (!question) throw new Error(`${id} missing`);
    question.req = true;
    question.req_index = 1;
  }
  const state = build(documents, "ru");
  setValue(state, "personalData.surname", "ru", "Иванов");
  expect(check(state, state.step, "ru")).toEqual([]);
});

test("check_other_required", () => {
  const documents = foldForm();
  const option = questions(documents).find((question) => question.id === "opt_1");
  if (!option) throw new Error("opt_1 missing");
  option.other_req = true;
  const state = build(documents, "ru");
  const otherId = state.questions.opt_1?.otherId;
  setValue(state, "opt_1", "ru", [otherId]);
  const empty = check(state, state.step, "ru");
  expect(empty.some((error) => error.id === "opt_1" && error.reason === "other")).toBe(true);
  const question = state.questions.opt_1;
  if (!question) throw new Error("opt_1 missing");
  question.otherText = "щит";
  const filled = check(state, state.step, "ru");
  expect(filled.some((error) => error.id === "opt_1" && error.reason === "other")).toBe(false);
});

test("check_counts_failure_uses_the_current_fields", async () => {
  const state = build(foldForm(), "ru");
  setValue(state, "personalData.name", "ru", "Иван");
  setValue(state, "personalData.companies.addresses.contact_phones.str_number", "ru", "+79637823887");
  state.errors = [{ id: "personalData.name", reason: "required" }];
  const step = [...state.step];
  const errors = await next(state, {
    counts: async () => {
      throw new Error("/api/registrations/counts 404");
    },
  });
  expect(errors[0]).toEqual({ id: "personalData.surname", reason: "required" });
  expect(state.errors[0]).toEqual({ id: "personalData.surname", reason: "required" });
  expect(state.step).toEqual(step);
});

test("check_counts_failure_still_advances", async () => {
  const documents = foldForm();
  documents.conf.forEach((screen, index) => {
    screen.screen_block = index + 1;
  });
  for (const question of questions(documents)) {
    if (documents.conf[0]?.body.includes(question)) question.req = false;
  }
  const state = build(documents, "ru");
  setValue(state, "personalData.companies.addresses.contact_phones.str_number", "ru", "+79637823887");
  let asked = false;
  await next(state, {
    counts: async () => {
      asked = true;
      throw new Error("/api/registrations/counts 404");
    },
  });
  expect(asked).toBe(true);
  expect(state.step).toEqual([1]);
  expect(state.errors).toEqual([]);
});

test("check_valid_step_advances", async () => {
  const documents = foldForm();
  documents.conf.forEach((screen, index) => {
    screen.screen_block = index + 1;
  });
  for (const question of questions(documents)) {
    if (documents.conf[0]?.body.includes(question)) question.req = false;
  }
  const state = build(documents, "ru");
  await next(state, { counts: async () => ({ count: 0 }) });
  expect(state.step).toEqual([1]);
});
