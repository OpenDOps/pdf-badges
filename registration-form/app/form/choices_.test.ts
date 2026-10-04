import type { FormDocuments } from "./client";
import { choiceOptions, choose, setOtherLine } from "./choices";
import { build, setValue } from "./engine";
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

function question(documents: FormDocuments, id: string): Record<string, unknown> {
  const found = questions(documents).find((item) => item.id === id);
  if (!found) throw new Error(`${id} missing`);
  return found;
}

function stored(state: ReturnType<typeof build>, id: string): unknown {
  const node = state.visitor[id];
  if (!isRecord(node)) throw new Error(`${id} missing`);
  return node.o_val;
}

test("choices_select_stores_one_id", () => {
  const documents = foldForm();
  const source = question(documents, "opt_6");
  source.select = true;
  source.chboxes = false;
  const state = build(documents, "ru");
  choose(state, "opt_6", 1);
  expect(stored(state, "opt_6")).toBe(1);
});

test("choices_checkboxes_store_a_list", () => {
  const state = build(foldForm(), "ru");
  choose(state, "opt_1", 0);
  choose(state, "opt_1", 3);
  expect(stored(state, "opt_1")).toEqual([0, 3]);
});

test("choices_single_select_stores_one", () => {
  const documents = foldForm();
  question(documents, "opt_1").single_select = true;
  const state = build(documents, "ru");
  choose(state, "opt_1", 0);
  choose(state, "opt_1", 3);
  expect(stored(state, "opt_1")).toBe(3);
});

test("choices_tree_keeps_depth", () => {
  const documents = foldForm();
  question(documents, "opt_3").tree = true;
  question(documents, "opt_3").chboxes = false;
  documents.vars.opt_3 = [
    { id: 1, v: { ru: "Корень", en: "Root" }, opts: [{ id: 2, v: { ru: "Ветка", en: "Child" } }] },
  ];
  const state = build(documents, "ru");
  const options = choiceOptions(state, "opt_3", "ru");
  expect(options.find((option) => option.id === 1)?.depth).toBe(0);
  expect(options.find((option) => option.id === 2)?.depth).toBe(1);
  choose(state, "opt_3", 2);
  expect(stored(state, "opt_3")).toEqual([2]);
});

test("choices_other_line", () => {
  const state = build(foldForm(), "ru");
  choose(state, "opt_1", 14);
  setOtherLine(state, "opt_1", "щит");
  expect(stored(state, "opt_1")).toEqual([14]);
  expect(state.questions.opt_1?.otherText).toBe("щит");
});

test("choices_binded_other_writes_the_sibling", () => {
  const documents = foldForm();
  question(documents, "opt_1").binded_other = "personalData.surname";
  const state = build(documents, "ru");
  setOtherLine(state, "opt_1", "щит");
  expect(state.visitor.personalData).toMatchObject({ surname: "щит" });
});

test("choices_unknown_id_is_rejected", () => {
  const closed = build(foldForm(), "ru");
  expect(() => choose(closed, "opt_6", 99)).toThrow(/unknown choice/);
  expect(stored(closed, "opt_6")).toEqual([]);

  const open = build(foldForm(), "ru");
  choose(open, "opt_1", 99);
  expect(stored(open, "opt_1")).toEqual([99]);
  expect(() => setValue(closed, "opt_6", "ru", 99)).toThrow(/unknown choice/);
});

test("choices_default_once_at_build", () => {
  const documents = foldForm();
  question(documents, "opt_6").dbenum_def = 1;
  const state = build(documents, "ru");
  expect(stored(state, "opt_6")).toBe(1);
  setValue(state, "opt_6", "ru", 0);
  expect(stored(state, "opt_6")).toBe(0);
});

test("choices_sort_by_id", () => {
  const documents = foldForm();
  documents.vars.opt_2 = [
    { id: 2, v: { ru: "Бета", en: "Beta" } },
    { id: 7, v: { ru: "Другое", en: "Other" } },
    { id: 3, v: { ru: "Яма", en: "Yard" }, default: true },
    { id: 1, v: { ru: "Альфа", en: "Alpha" } },
  ];
  const state = build(documents, "ru");
  expect(choiceOptions(state, "opt_2", "ru").map((option) => option.label)).toEqual(["Альфа", "Бета", "Яма", "Другое"]);
});

test("choices_real_lists_follow_id", () => {
  const state = build(foldForm(), "ru");
  for (const id of ["opt_1", "opt_2", "opt_3", "opt_4", "opt_5"]) {
    const options = choiceOptions(state, id, "ru");
    const ids = options.map((option) => option.id);
    expect(ids).toEqual(ids.slice().sort((left, right) => left - right));
    expect(options.at(-1)?.label).toBe("Другое");
  }
});
