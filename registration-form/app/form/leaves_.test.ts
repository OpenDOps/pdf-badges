import { foldForm } from "./fold";
import { leafAt } from "./leaves";

const form = foldForm();

test("leaves_string", () => {
  const leaf = leafAt(form.struct, form.vars, "personalData.companies.name");
  expect(leaf.kind).toBe("parametric");
  expect(leaf.link).toBe("str");
});

test("leaves_enum_name", () => {
  const leaf = leafAt(form.struct, form.vars, "personalData.companies.addresses.country_db");
  expect(leaf.kind).toBe("enum");
  expect(leaf.enumName).toBe("country");
});

test("leaves_option", () => {
  const leaf = leafAt(form.struct, form.vars, "opt_1");
  expect(leaf.kind).toBe("option");
  expect(leaf.optionKey).toBe("opt_1");
});

test("leaves_phone_filter", () => {
  const leaf = leafAt(form.struct, form.vars, "personalData.companies.addresses.contact_phones");
  expect(leaf.kind).toBe("repeating");
  expect(leaf.relPropUnique).toEqual(["primary"]);
  expect(leaf.filter).toEqual([{ phone_type: 2 }]);
});

test("leaves_missing_path", () => {
  expect(() => leafAt(form.struct, form.vars, "no.such.field")).toThrow(/no\.such\.field/);
});
