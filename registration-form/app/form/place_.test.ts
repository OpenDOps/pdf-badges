import { readFile } from "node:fs/promises";
import path from "node:path";

import { FormClient } from "./client";
import { build, setValue } from "./engine";
import { FILE_LIMIT, settleFiles } from "./files";
import { foldForm } from "./fold";
import { setPhoneCountry } from "./phones";
import { loadPlaces, placeKey, refreshPlace, searchCities, setCity, setCountry } from "./place";

const ADDRESS = "personalData.companies.addresses";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function publicFetch() {
  const urls: string[] = [];
  let failFrom = Number.POSITIVE_INFINITY;
  let gate: Promise<void> = Promise.resolve();
  const impl: typeof fetch = async (input) => {
    const raw = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    const pathname = raw.startsWith("/") ? raw : new URL(raw).pathname;
    urls.push(pathname);
    if (pathname.includes("/city_") || pathname.includes("city_phone_mask.json")) await gate;
    if (urls.length >= failFrom) return new Response("fail", { status: 500 });
    const file = path.join(process.cwd(), "public", ...pathname.split("/").filter(Boolean));
    const body = await readFile(file);
    return new Response(body, { status: 200, headers: { "Content-Type": "application/json" } });
  };
  return {
    impl,
    urls,
    failAfter(count: number) {
      failFrom = count + 1;
    },
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

async function until(predicate: () => boolean) {
  for (let attempt = 0; attempt < 50; attempt += 1) {
    if (predicate()) return;
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
  throw new Error("timed out");
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

function idOf(row: Record<string, unknown>, field: string): unknown {
  const node = row[field];
  return isRecord(node) ? node.e_val : undefined;
}

function clientFor() {
  const transport = publicFetch();
  return { transport, client: new FormClient(transport.impl), state: build(foldForm(), "ru") };
}

test("place_country_loads_at_build", async () => {
  const { transport, client, state } = clientFor();
  await loadPlaces(state, client);
  expect(transport.urls).toEqual(["/dbenums/country_ru.json"]);
});

test("place_region_waits_for_country", async () => {
  const { transport, client, state } = clientFor();
  const row = address(state);
  const country = row.country_db;
  if (isRecord(country)) country.e_val = null;
  await loadPlaces(state, client);
  expect(idOf(row, "region_db")).toBeNull();
  expect(transport.urls.some((url) => url.includes("/region_"))).toBe(false);
  await setCountry(state, client, ADDRESS, 219);
  expect(transport.urls).toContain("/dbenums/country-219/region_ru.json");
  expect(idOf(row, "region_db")).toBe(3948);
});

test("place_city_waits_for_both", async () => {
  const { transport, client, state } = clientFor();
  const row = address(state);
  const country = row.country_db;
  if (isRecord(country)) country.e_val = null;
  await loadPlaces(state, client);
  expect(transport.urls.some((url) => url.includes("/city_"))).toBe(false);
  await setCountry(state, client, ADDRESS, 219);
  expect(transport.urls).toContain("/dbenums/country-219/region-3948/city_ru.json");
  expect(idOf(row, "city_db")).toBe(17849);
});

test("place_country_change_sets_defaults", async () => {
  const { client, state } = clientFor();
  const row = address(state);
  await setCountry(state, client, ADDRESS, 219);
  await setCity(state, client, ADDRESS, 18260);
  expect(idOf(row, "city_db")).toBe(18260);
  await setCountry(state, client, ADDRESS, 224);
  expect(idOf(row, "region_db")).toBe(1750);
  expect(idOf(row, "city_db")).toBe(25054);
});

test("place_region_is_not_a_control", async () => {
  const { client, state } = clientFor();
  await loadPlaces(state, client);
  const place = state.places[placeKey(ADDRESS, 0)];
  expect(place?.region?.ui).toBe(false);
  expect(place?.city?.ui).toBe(true);
  const before = idOf(address(state), "region_db");
  expect(() => setValue(state, `${ADDRESS}.region_db`, "ru", 1617)).toThrow(/region/);
  expect(idOf(address(state), "region_db")).toBe(before);
});

test("place_city_sets_the_region", async () => {
  const { client, state } = clientFor();
  const row = address(state);
  await setCity(state, client, ADDRESS, 18260);
  expect(idOf(row, "city_db")).toBe(18260);
  expect(idOf(row, "region_db")).toBe(1617);
  expect(idOf(row, "country_db")).toBe(219);
});

test("place_second_address_stays", async () => {
  const { client, state } = clientFor();
  const first = address(state);
  const personal = state.visitor.personalData;
  if (!isRecord(personal) || !Array.isArray(personal.companies)) throw new Error("companies missing");
  const company = personal.companies[0];
  if (!isRecord(company) || !Array.isArray(company.addresses)) throw new Error("addresses missing");
  const second = structuredClone(first);
  second.jvrel_prop = { primary: false };
  const country = second.country_db;
  const region = second.region_db;
  const city = second.city_db;
  if (!isRecord(country) || !isRecord(region) || !isRecord(city)) throw new Error("place missing");
  country.e_val = 224;
  region.e_val = 1750;
  city.e_val = 25054;
  company.addresses.push(second);
  await setCountry(state, client, ADDRESS, 219, 0);
  expect(idOf(address(state, 1), "country_db")).toBe(224);
  expect(idOf(address(state, 1), "region_db")).toBe(1750);
  expect(idOf(address(state, 1), "city_db")).toBe(25054);
  expect(idOf(address(state, 0), "region_db")).toBe(3948);
  expect(idOf(address(state, 0), "city_db")).toBe(17849);
});

test("place_default_only_when_null", async () => {
  const filled = clientFor();
  const filledCountry = address(filled.state).country_db;
  if (isRecord(filledCountry)) filledCountry.e_val = 224;
  await loadPlaces(filled.state, filled.client);
  expect(idOf(address(filled.state), "country_db")).toBe(224);

  const empty = clientFor();
  const emptyCountry = address(empty.state).country_db;
  if (isRecord(emptyCountry)) emptyCountry.e_val = null;
  await loadPlaces(empty.state, empty.client);
  expect(idOf(address(empty.state), "country_db")).toBe(219);
});

test("place_cities_load_ten_at_a_time", async () => {
  const { transport, client, state } = clientFor();
  const release = transport.hold();
  const pending = setCountry(state, client, ADDRESS, 219);
  await until(() => transport.urls.filter((url) => url.includes("/city_")).length === FILE_LIMIT);
  const cities = transport.urls.filter((url) => url.includes("/city_"));
  expect(cities).toHaveLength(FILE_LIMIT);
  expect(cities[0]).toBe("/dbenums/country-219/region-3948/city_ru.json");
  expect(searchCities(state, ADDRESS, "Акутиха")).toEqual([]);
  const phones = setPhoneCountry(state, client, ADDRESS, 219);
  await until(() => transport.urls.some((url) => url.includes("/country_phone_mask.json")));
  expect(transport.urls.some((url) => url.includes("city_phone_mask.json"))).toBe(false);
  release();
  await pending;
  await phones;
  await settleFiles(state);
  const loaded = transport.urls.filter((url) => url.includes("/city_"));
  expect(new Set(loaded).size).toBe(loaded.length);
  const before = loaded.length;
  await setCountry(state, client, ADDRESS, 219);
  expect(transport.urls.filter((url) => url.includes("/city_")).length).toBe(before);
  expect(searchCities(state, ADDRESS, "Акутиха")).toEqual([{ id: 18260, regionId: 1617, name: "Акутиха", weight: 1 }]);
});

test("place_failed_list_keeps_the_previous", async () => {
  const { transport, client, state } = clientFor();
  await setCountry(state, client, ADDRESS, 219);
  await settleFiles(state);
  const place = state.places[placeKey(ADDRESS, 0)];
  const previous = place?.region?.list;
  expect(previous?.length).toBeGreaterThan(0);
  transport.failAfter(transport.urls.length);
  await refreshPlace(state, client, ADDRESS, 0, "region");
  expect(place?.region?.list).toBe(previous);
  expect(place?.region?.error).toBeInstanceOf(Error);
});
