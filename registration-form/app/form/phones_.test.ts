import { readFile } from "node:fs/promises";
import path from "node:path";

import { FormClient } from "./client";
import { FILE_LIMIT, settleFiles } from "./files";
import { build, check, next } from "./engine";
import { foldForm } from "./fold";
import { leafAt } from "./leaves";
import {
  addPhone,
  loadPhoneRegion,
  setPhoneCountry,
  phoneCallingCode,
  phoneCountryMasks,
  phoneMask,
  phoneText,
  removePhone,
  setNational,
  setPhonePrimary,
  shownPhones,
} from "./phones";
import { setCountry } from "./place";

const ADDRESS = "personalData.companies.addresses";
const PHONES = "personalData.companies.addresses.contact_phones";
const PHONE = "personalData.companies.addresses.contact_phones.str_number";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function publicFetch() {
  const urls: string[] = [];
  let gate: Promise<void> = Promise.resolve();
  const impl: typeof fetch = async (input) => {
    const raw = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    const pathname = raw.startsWith("/") ? raw : new URL(raw).pathname;
    urls.push(pathname);
    if (pathname.includes("/city_phone_mask.json")) await gate;
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
  return {
    impl,
    urls,
    hold() {
      let release: () => void = () => {};
      gate = new Promise((resolve) => {
        release = resolve;
      });
      return () => {
        release();
        gate = Promise.resolve();
      };
    },
  };
}

function address(state: ReturnType<typeof build>, index = 0): Record<string, unknown> {
  const personal = state.visitor.personalData;
  if (!isRecord(personal) || !Array.isArray(personal.companies)) throw new Error("companies missing");
  const company = personal.companies[0];
  if (!isRecord(company) || !Array.isArray(company.addresses)) throw new Error("addresses missing");
  const row = company.addresses[index];
  if (!isRecord(row)) throw new Error("address missing");
  return row;
}

function phoneRow(row: Record<string, unknown>): Record<string, unknown> {
  const phones = row.contact_phones;
  if (!Array.isArray(phones) || !isRecord(phones[0])) throw new Error("phone missing");
  return phones[0];
}

function clientFor() {
  const transport = publicFetch();
  return { transport, client: new FormClient(transport.impl), state: build(foldForm(), "ru") };
}

async function until(predicate: () => boolean) {
  for (let attempt = 0; attempt < 50; attempt += 1) {
    if (predicate()) return;
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
  throw new Error("timed out");
}

async function untilMask(state: ReturnType<typeof build>, expected: string, index = 0) {
  await until(() => phoneMask(state, ADDRESS, index) === expected);
  expect(phoneMask(state, ADDRESS, index)).toBe(expected);
}

test("phones_text_follows_the_mask", () => {
  expect(phoneText("+7 (ddd) ddd-dd-dd", "+7", "", true)).toBe("(___) ___-__-__");
  expect(phoneText("+7 (ddd) ddd-dd-dd", "+7", "", false)).toBe("");
  expect(phoneText("+7 (ddd) ddd-dd-dd", "+7", "495", true)).toBe("(495) ___-__-__");
  expect(phoneText("+7 (495) ddd-dd-dd", "+7", "495123", true)).toBe("(495) 123-__-__");
  expect(phoneText("+7 (495) ddd-dd-dd", "+7", "4951234567", false)).toBe("(495) 123-45-67");
});

test("phones_mask_follows_this_country", async () => {
  const { client, state } = clientFor();
  await setPhoneCountry(state, client, ADDRESS, 219);
  expect(phoneCountryMasks(state, ADDRESS)).toBe("+7 (ddd) ddd-dd-dd");
  await setPhoneCountry(state, client, ADDRESS, 222);
  expect(phoneCountryMasks(state, ADDRESS)).toBe("+7 (ddd) ddd dddd; +7 (dddd) dd dd dd; +7 (ddddd) d dd dd");
}, 20000);

test("phones_region_masks_load_for_the_country", async () => {
  const loaded = clientFor();
  const before = loaded.transport.urls.length;
  await setPhoneCountry(loaded.state, loaded.client, ADDRESS, 219);
  await settleFiles(loaded.state);
  const regionUrls = loaded.transport.urls.slice(before).filter((url) => url.includes("/city_phone_mask.json"));
  expect(regionUrls[0]).toBe("/dbenums/country-219/region-3948/city_phone_mask.json");
  expect(regionUrls).toContain("/dbenums/country-219/region-1651/city_phone_mask.json");
  expect(new Set(regionUrls).size).toBe(85);

  const held = clientFor();
  await settleFiles(held.state);
  const release = held.transport.hold();
  await setPhoneCountry(held.state, held.client, ADDRESS, 219);
  expect(phoneCountryMasks(held.state, ADDRESS)).toBe("+7 (ddd) ddd-dd-dd");
  const started = held.transport.urls.filter((url) => url.includes("/city_phone_mask.json"));
  expect(started[0]).toBe("/dbenums/country-219/region-3948/city_phone_mask.json");
  expect(started).toHaveLength(FILE_LIMIT);
  setNational(held.state, ADDRESS, "495");
  expect(phoneMask(held.state, ADDRESS)).toBe("+7 (ddd) ddd-dd-dd");
  release();
  await untilMask(held.state, "+7 (495) ddd-dd-dd");
  await settleFiles(held.state);
  await loadPhoneRegion(held.state, held.client, ADDRESS, 1651);
  expect(held.transport.urls.filter((url) => url.includes("/region-1651/city_phone_mask.json"))).toHaveLength(1);
}, 20000);

test("phones_mask_uses_the_region_file", async () => {
  const { client, state } = clientFor();
  await setPhoneCountry(state, client, ADDRESS, 221);
  setNational(state, ADDRESS, "1715");
  await untilMask(state, "+375 (1715) d-dd-dd");
  expect(phoneRow(address(state)).str_number).toBe("+3751715");
  const step = state.step.slice();
  const lookedUp: string[] = [];
  const errors = await next(state, {
    counts: async (kind, value) => {
      lookedUp.push(`${kind}:${value}`);
      return { count: 1 };
    },
  });
  expect(lookedUp).toContain("phone:+3751715");
  expect(errors).toContainEqual({ id: PHONE, reason: "unique" });
  expect(state.step).toEqual(step);
  setNational(state, ADDRESS, "171");
  expect(phoneMask(state, ADDRESS)).toBe("+375 (17) ddd-dd-dd");
  setNational(state, ADDRESS, "17");
  expect(phoneMask(state, ADDRESS)).toBe("+375 (17) ddd-dd-dd");
}, 20000);

test("phones_five_digit_code_switches_the_mask", async () => {
  const { client, state } = clientFor();
  await setPhoneCountry(state, client, ADDRESS, 219);
  setNational(state, ADDRESS, "8314");
  expect(phoneMask(state, ADDRESS)).toBe("+7 (ddd) ddd-dd-dd");
  setNational(state, ADDRESS, "83147");
  await untilMask(state, "+7 (83147) d-dd-dd");
  setNational(state, ADDRESS, "8314");
  expect(phoneMask(state, ADDRESS)).toBe("+7 (ddd) ddd-dd-dd");
}, 20000);

test("phones_mask_follows_leading_digits", async () => {
  const moscow = clientFor();
  await setPhoneCountry(moscow.state, moscow.client, ADDRESS, 219);
  setNational(moscow.state, ADDRESS, "495");
  await untilMask(moscow.state, "+7 (495) ddd-dd-dd");
  setNational(moscow.state, ADDRESS, "4951");
  expect(phoneMask(moscow.state, ADDRESS)).toBe("+7 (495) ddd-dd-dd");

  const astana = clientFor();
  await setPhoneCountry(astana.state, astana.client, ADDRESS, 222);
  await loadPhoneRegion(astana.state, astana.client, ADDRESS, 1720);
  setNational(astana.state, ADDRESS, "717");
  await untilMask(astana.state, "+7 (717) ddd dddd");
  setNational(astana.state, ADDRESS, "7172");
  expect(phoneMask(astana.state, ADDRESS)).toBe("+7 (717) ddd dddd");

  const turkestan = clientFor();
  await setPhoneCountry(turkestan.state, turkestan.client, ADDRESS, 222);
  await loadPhoneRegion(turkestan.state, turkestan.client, ADDRESS, 1733);
  setNational(turkestan.state, ADDRESS, "7253");
  await untilMask(turkestan.state, "+7 (ddddd) d dd dd");
  setNational(turkestan.state, ADDRESS, "72533");
  expect(phoneMask(turkestan.state, ADDRESS)).toBe("+7 (72533) d dd dd");
}, 20000);

test("phones_no_country_is_digits", async () => {
  const { client, state } = clientFor();
  await setPhoneCountry(state, client, ADDRESS, null);
  setNational(state, ADDRESS, "9031234567");
  await setCountry(state, client, ADDRESS, 372);
  expect(phoneCallingCode(state, ADDRESS)).toBe("");
  expect(phoneRow(address(state)).str_number).toBe("9031234567");
});

test("phones_country_change_is_local", async () => {
  const { client, state } = clientFor();
  const personal = state.visitor.personalData;
  if (!isRecord(personal) || !Array.isArray(personal.companies)) throw new Error("companies missing");
  const company = personal.companies[0];
  if (!isRecord(company) || !Array.isArray(company.addresses)) throw new Error("addresses missing");
  const second = structuredClone(address(state));
  second.jvrel_prop = { primary: false };
  company.addresses.push(second);
  await setPhoneCountry(state, client, ADDRESS, 219, 0);
  setNational(state, ADDRESS, "4951234567", 0);
  await setPhoneCountry(state, client, ADDRESS, 221, 1);
  setNational(state, ADDRESS, "171512345", 1);
  await untilMask(state, "+375 (1715) d-dd-dd", 1);
  const keptNumber = phoneRow(address(state, 1)).str_number;
  const keptMask = phoneMask(state, ADDRESS, 1);
  const keptMask0 = phoneMask(state, ADDRESS, 0);
  await setCountry(state, client, ADDRESS, 372, 0);
  expect(phoneRow(address(state, 0)).str_number).toBe("+74951234567");
  expect(phoneCallingCode(state, ADDRESS, 0)).toBe("+7");
  expect(phoneMask(state, ADDRESS, 0)).toBe(keptMask0);
  expect(phoneRow(address(state, 1)).str_number).toBe(keptNumber);
  expect(phoneMask(state, ADDRESS, 1)).toBe(keptMask);
  await setPhoneCountry(state, client, ADDRESS, 372, 0);
  expect(phoneRow(address(state, 0)).str_number).toBe("+14951234567");
  expect(phoneMask(state, ADDRESS, 0)).toBe("+1 (ddd) ddd-dddd");
  expect(phoneRow(address(state, 1)).str_number).toBe(keptNumber);
  expect(phoneMask(state, ADDRESS, 1)).toBe(keptMask);
}, 20000);

test("phones_address_country_loads_masks_in_background", async () => {
  const { client, state } = clientFor();
  const pending = setCountry(state, client, ADDRESS, 219);
  expect(phoneCallingCode(state, ADDRESS)).toBe("");
  await pending;
  await until(() => phoneCallingCode(state, ADDRESS) === "+7" && phoneCountryMasks(state, ADDRESS) === "+7 (ddd) ddd-dd-dd");
  expect(phoneRow(address(state)).str_number).toBe("");
  setNational(state, ADDRESS, "4951234567");
  expect(phoneRow(address(state)).str_number).toBe("+74951234567");
  await setCountry(state, client, ADDRESS, 372);
  expect(phoneRow(address(state)).str_number).toBe("+74951234567");
  expect(phoneCallingCode(state, ADDRESS)).toBe("+7");
}, 20000);

test("phones_stores_str_number", async () => {
  const { client, state } = clientFor();
  await setPhoneCountry(state, client, ADDRESS, 219);
  setNational(state, ADDRESS, "4951234567");
  const row = phoneRow(address(state));
  expect(row.str_number).toBe("+74951234567");
  expect(row.country_code).toBeUndefined();
  expect(row.city_or_operator).toBeUndefined();
  expect(row.number).toBeUndefined();
}, 20000);

function phoneFailed(state: ReturnType<typeof build>): boolean {
  return check(state, state.step, "ru").some((error) => error.id === PHONE && error.reason === "required");
}

test("phones_no_digits_is_empty", async () => {
  const russia = clientFor();
  await setPhoneCountry(russia.state, russia.client, ADDRESS, 219);
  phoneRow(address(russia.state)).str_number = "+7";
  expect(phoneFailed(russia.state)).toBe(true);
  phoneRow(address(russia.state)).str_number = "+7123";
  expect(phoneFailed(russia.state)).toBe(false);

  const belarus = clientFor();
  await setPhoneCountry(belarus.state, belarus.client, ADDRESS, 221);
  phoneRow(address(belarus.state)).str_number = "+375";
  expect(phoneFailed(belarus.state)).toBe(true);
  phoneRow(address(belarus.state)).str_number = "+3751";
  expect(phoneFailed(belarus.state)).toBe(false);

  const united = clientFor();
  await setPhoneCountry(united.state, united.client, ADDRESS, 372);
  phoneRow(address(united.state)).str_number = "+1";
  expect(phoneFailed(united.state)).toBe(true);
});

test("phones_filter_splits_lists", () => {
  const documents = foldForm();
  const state = build(documents, "ru");
  const leaf = leafAt(documents.struct, documents.vars, PHONES);
  const phones = address(state).contact_phones;
  if (!Array.isArray(phones)) throw new Error("phones missing");
  phones.push({ jvlink_id: -1, jvrel_prop: { primary: false }, str_number: "", phone_type: 1 });
  const fax = [{ phone_type: 1 }];
  const mobile = shownPhones(state, PHONES, leaf.filter);
  const faxes = shownPhones(state, PHONES, fax);
  expect(mobile.map((row) => row.phone_type)).toEqual([2]);
  expect(faxes.map((row) => row.phone_type)).toEqual([1]);
});

test("phones_last_row_stays", () => {
  const documents = foldForm();
  const state = build(documents, "ru");
  const leaf = leafAt(documents.struct, documents.vars, PHONES);
  const only = shownPhones(state, PHONES, leaf.filter)[0];
  if (!only) throw new Error("phone missing");
  removePhone(state, PHONES, leaf.filter, only);
  expect(shownPhones(state, PHONES, leaf.filter)).toHaveLength(1);
});

test("phones_primary_is_one", () => {
  const documents = foldForm();
  const state = build(documents, "ru");
  const leaf = leafAt(documents.struct, documents.vars, PHONES);
  const added = addPhone(state, PHONES, leaf.filter);
  expect(added.jvlink_id).toBe(-1);
  const prop = added.jvrel_prop;
  if (!isRecord(prop)) throw new Error("primary missing");
  expect(prop.primary).toBe(false);
  setPhonePrimary(state, PHONES, leaf.filter, added);
  const rows = shownPhones(state, PHONES, leaf.filter);
  const primaries = rows.filter((row) => isRecord(row.jvrel_prop) && row.jvrel_prop.primary === true);
  expect(primaries).toEqual([added]);
});

test("phones_unique_num", async () => {
  const { transport, client, state } = clientFor();
  const phones = address(state).contact_phones;
  if (!Array.isArray(phones)) throw new Error("phones missing");
  const row = phones[0];
  if (!isRecord(row)) throw new Error("phone missing");
  row.str_number = "+74951234567";
  const step = state.step.slice();
  const errors = await next(state, client);
  expect(errors).toContainEqual({ id: PHONE, reason: "unique" });
  expect(state.step).toEqual(step);
  expect(transport.urls.some((url) => decodeURIComponent(url).includes("+74951234567"))).toBe(true);
});
