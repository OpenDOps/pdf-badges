import { build, setValue } from "./engine";
import { foldForm } from "./fold";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

test("values_same_id_is_one_question", () => {
  const documents = foldForm();
  const surname = documents.conf[0]?.body[0];
  documents.conf[2]?.body.push(surname);
  const state = build(documents, "ru");
  const matches = Object.values(state.questions).filter((question) => question.id === "personalData.surname");
  expect(matches).toHaveLength(1);
});

test("values_string_writes_the_locale", () => {
  const state = build(foldForm(), "ru");
  setValue(state, "personalData.companies.name", "ru", "Анна");
  setValue(state, "personalData.companies.name", "en", "Ann");
  const personal = state.visitor.personalData;
  if (!isRecord(personal) || !Array.isArray(personal.companies)) throw new Error("companies missing");
  const company = personal.companies[0];
  if (!isRecord(company) || !isRecord(company.name)) throw new Error("name missing");
  const en = company.name.en;
  const ru = company.name.ru;
  if (!isRecord(en) || !isRecord(ru)) throw new Error("locale missing");
  expect(en.str).toBe("Ann");
  expect(ru.str).toBe("Анна");
});

test("values_enum_empty_is_null", () => {
  const state = build(foldForm(), "ru");
  const personal = state.visitor.personalData;
  if (!isRecord(personal) || !Array.isArray(personal.companies)) throw new Error("companies missing");
  const company = personal.companies[0];
  if (!isRecord(company) || !Array.isArray(company.addresses)) throw new Error("addresses missing");
  const address = company.addresses[0];
  if (!isRecord(address)) throw new Error("address missing");
  if (!isRecord(address.city_db) || !isRecord(address.region_db) || !isRecord(address.country_db)) {
    throw new Error("place missing");
  }
  expect(address.city_db.e_val).toBeNull();
  expect(address.region_db.e_val).toBeNull();
  expect(address.country_db.e_val).toBe(219);
});

test("values_checkboxes_write_a_list", () => {
  const state = build(foldForm(), "ru");
  setValue(state, "opt_1", "ru", [0, 3]);
  const option = state.visitor.opt_1;
  if (!isRecord(option)) throw new Error("opt_1 missing");
  expect(option.o_val).toEqual([0, 3]);
});

test("values_single_select_writes_one_id", () => {
  const state = build(foldForm(), "ru");
  setValue(state, "opt_6", "ru", 1);
  const option = state.visitor.opt_6;
  if (!isRecord(option)) throw new Error("opt_6 missing");
  expect(option.o_val).toBe(1);
});

test("values_repeating_writes_the_primary", () => {
  const state = build(foldForm(), "ru");
  const personal = state.visitor.personalData;
  if (!isRecord(personal) || !Array.isArray(personal.emails)) throw new Error("emails missing");
  personal.emails.push({ jvlink_id: -2, jvrel_prop: { primary: false }, email: "other@x.c" });
  setValue(state, "personalData.emails.email", "ru", "a@b.c");
  const primary = personal.emails[0];
  const other = personal.emails[1];
  if (!isRecord(primary) || !isRecord(other)) throw new Error("email row missing");
  expect(primary.email).toBe("a@b.c");
  expect(other.email).toBe("other@x.c");
});

test("values_hidden_stays_in_the_map", () => {
  const state = build(foldForm(), "ru");
  expect(state.questions["personalData.companies.addresses.region_db"]?.hidden).toBe(true);
  expect(state.questions["personalData.addresses.country_db"]?.hidden).toBe(true);
});
