import type { FormDocuments } from "./client";
import { back, build, next, setLocale, setValue } from "./engine";
import { foldForm } from "./fold";

const client = { counts: async () => ({ count: 0 }) };

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

function separate(documents: FormDocuments) {
  documents.conf.forEach((screen, index) => {
    screen.screen_block = index + 1;
  });
  for (const question of questions(documents)) question.req = false;
}

function surname(state: ReturnType<typeof build>): string {
  const personal = state.visitor.personalData;
  if (!isRecord(personal) || typeof personal.surname !== "string") throw new Error("surname missing");
  return personal.surname;
}

test("walk_unconditional_next", async () => {
  const documents = foldForm();
  separate(documents);
  const state = build(documents, "ru");
  await next(state, client);
  expect(state.ended).toBe(false);
  expect(state.step).toEqual([1]);
});

test("walk_and_needs_every_clause", async () => {
  const documents = foldForm();
  separate(documents);
  documents.conf[0].next = [
    {
      step: 2,
      conditions: {
        operator: "and",
        expression: [
          { q_id: "personalData.surname", value: "Иванов" },
          { q_id: "personalData.name", value: "Анна" },
        ],
      },
    },
    { step: 3, conditions: { operator: "and", expression: [{ q_id: "personalData.surname", value: "Иванов" }] } },
  ];
  const state = build(documents, "ru");
  setValue(state, "personalData.surname", "ru", "Иванов");
  await next(state, client);
  expect(state.step).toEqual([3]);
});

test("walk_or_passes_on_one", async () => {
  const documents = foldForm();
  separate(documents);
  documents.conf[0].next = [
    {
      step: 2,
      conditions: {
        operator: "or",
        expression: [
          { q_id: "personalData.surname", value: "нет" },
          { q_id: "personalData.name", value: "Ann" },
        ],
      },
    },
  ];
  const state = build(documents, "ru");
  setValue(state, "personalData.name", "ru", "Ann");
  await next(state, client);
  expect(state.step).toEqual([2]);
});

test("walk_other_operator_is_and", async () => {
  const documents = foldForm();
  separate(documents);
  documents.conf[0].next = [
    {
      step: 2,
      conditions: {
        operator: "xor",
        expression: [
          { q_id: "personalData.surname", value: "Иванов" },
          { q_id: "personalData.name", value: "Анна" },
        ],
      },
    },
    { step: 3 },
  ];
  const state = build(documents, "ru");
  setValue(state, "personalData.surname", "ru", "Иванов");
  await next(state, client);
  expect(state.step).toEqual([3]);
});

test("walk_list_contains_the_value", async () => {
  const documents = foldForm();
  separate(documents);
  documents.conf[0].next = [
    { step: 2, conditions: { operator: "and", expression: [{ q_id: "opt_1", value: 5 }] } },
  ];
  const state = build(documents, "ru");
  setValue(state, "opt_1", "ru", [1, 5]);
  await next(state, client);
  expect(state.step).toEqual([2]);
});

test("walk_screen_block_is_one_step", async () => {
  const documents = foldForm();
  documents.conf[2].screen_block = 2;
  documents.conf[3].screen_block = 3;
  documents.conf[0].next = [{ step: 9 }];
  for (const question of questions(documents)) question.req = false;
  const state = build(documents, "ru");
  expect(state.step).toEqual([0, 1]);
  await next(state, client);
  expect(state.step).toEqual([2]);
});

test("walk_back_pops", async () => {
  const documents = foldForm();
  separate(documents);
  const state = build(documents, "ru");
  setValue(state, "personalData.surname", "ru", "Иванов");
  await next(state, client);
  back(state);
  expect(state.step).toEqual([0]);
  expect(surname(state)).toBe("Иванов");
});

test("walk_locale_skips_and_keeps_answers", async () => {
  const documents = foldForm();
  separate(documents);
  documents.conf[1].for_locales = ["ru"];
  const state = build(documents, "ru");
  setValue(state, "personalData.surname", "ru", "Иванов");
  await next(state, client);
  expect(state.step).toEqual([1]);
  setLocale(state, "en");
  expect(state.step).toEqual([2]);
  expect(surname(state)).toBe("Иванов");
});

test("walk_end_is_minus_one", async () => {
  const documents = foldForm();
  separate(documents);
  const state = build(documents, "ru");
  await next(state, client);
  await next(state, client);
  await next(state, client);
  expect(state.step).toEqual([3]);
  await next(state, client);
  expect(state.ended).toBe(true);
});

test("walk_payment_is_the_end", async () => {
  const documents = foldForm();
  separate(documents);
  documents.conf[0].next = [{ step: 1, payment: true }];
  const state = build(documents, "ru");
  await next(state, client);
  expect(state.ended).toBe(true);
  expect(state.step).toEqual([0]);
});
