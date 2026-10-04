import { readFile } from "node:fs/promises";
import path from "node:path";

import { FormClient, enumPath } from "./client";
import { foldForm } from "./fold";

function publicFetch() {
  const urls: string[] = [];
  let failFrom = Number.POSITIVE_INFINITY;
  let gate: Promise<void> = Promise.resolve();
  const impl: typeof fetch = async (input) => {
    const raw = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    const pathname = raw.startsWith("/") ? raw : new URL(raw).pathname;
    urls.push(pathname);
    await gate;
    if (urls.length >= failFrom) return new Response("fail", { status: 500 });
    if (pathname === "/forms/form.json") {
      return new Response(JSON.stringify(foldForm()), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      });
    }
    const file = path.join(process.cwd(), "public", ...pathname.split("/").filter(Boolean));
    const body = await readFile(file);
    return new Response(body, {
      status: 200,
      headers: { "Content-Type": "application/json" },
    });
  };
  return {
    impl,
    urls,
    hold() {
      let release: () => void = () => {};
      gate = new Promise((resolve) => {
        release = resolve;
      });
      return () => release();
    },
    failAfter(count: number) {
      failFrom = count + 1;
    },
  };
}

function varNames(vars: Record<string, unknown>): string[] {
  return Object.values(vars).flatMap((value) => {
    if (value && typeof value === "object" && "name" in value && typeof value.name === "string") {
      return [value.name];
    }
    return [];
  });
}

test("documents_load_returns_the_form", async () => {
  const transport = publicFetch();
  const client = new FormClient(transport.impl);
  const form = await client.load();

  expect(varNames(form.vars)).toEqual(expect.arrayContaining(["country", "region", "city"]));
  expect(Array.isArray(form.conf)).toBe(true);
  expect(form.conf[0]?.advice.ru).toBe("Личные данные");
  expect(form.model.uniqueId).toBe("FJVFOMEICW");
  expect(form.struct).toBeTruthy();
  expect(form.settings).toEqual({});
  expect(form.barcodes).toBeTruthy();
  expect(JSON.stringify(form.model)).not.toContain("jv_rights");
  expect(transport.urls).toEqual(["/forms/form.json"]);
});

test("documents_second_country_ask_is_cached", async () => {
  const transport = publicFetch();
  const client = new FormClient(transport.impl);

  const first = await client.enums("ru", "country", {});
  const second = await client.enums("ru", "country", {});

  expect(transport.urls).toEqual(["/dbenums/country_ru.json"]);
  expect(second.list).toEqual(first.list);
  expect(second.list).not.toBe(first.list);
  first.list.splice(0, first.list.length);
  const third = await client.enums("ru", "country", {});
  expect(third.list).toContainEqual(expect.objectContaining({ id: 219, v: "Россия" }));
  expect(first.list).toEqual([]);
  expect(first.error).toBeNull();
});

test("documents_concurrent_asks_share_one_request", async () => {
  const transport = publicFetch();
  const release = transport.hold();
  const client = new FormClient(transport.impl);

  const first = client.enums("ru", "country", {});
  const second = client.enums("ru", "country", {});
  expect(transport.urls).toEqual(["/dbenums/country_ru.json"]);
  release();

  const [a, b] = await Promise.all([first, second]);
  expect(a.list).toEqual(b.list);
  expect(a.list).toContainEqual(expect.objectContaining({ id: 219, v: "Россия" }));
  expect(transport.urls).toEqual(["/dbenums/country_ru.json"]);
});

test("documents_failed_refresh_keeps_the_list", async () => {
  const transport = publicFetch();
  const client = new FormClient(transport.impl);

  const first = await client.enums("ru", "country", {});
  transport.failAfter(1);
  const refreshed = await client.enums("ru", "country", {}, { refresh: true });
  const again = await client.enums("ru", "country", {});

  expect(refreshed.error).toBeInstanceOf(Error);
  expect(refreshed.list).toEqual(first.list);
  expect(again.list).toEqual(first.list);
  expect(again.error).toBeNull();
  expect(transport.urls).toEqual(["/dbenums/country_ru.json", "/dbenums/country_ru.json"]);
});

test("documents_failed_refresh_shares_the_result", async () => {
  const transport = publicFetch();
  const client = new FormClient(transport.impl);
  const first = await client.enums("ru", "country", {});
  const release = transport.hold();
  transport.failAfter(transport.urls.length);
  const refresh = client.enums("ru", "country", {}, { refresh: true });
  const joined = client.enums("ru", "country", {});
  expect(transport.urls).toEqual(["/dbenums/country_ru.json", "/dbenums/country_ru.json"]);
  release();
  const [failed, shared] = await Promise.all([refresh, joined]);
  expect(shared).toBe(failed);
  expect(failed.error).toBeInstanceOf(Error);
  expect(failed.list).toEqual(first.list);
});

test("documents_enum_path_rejects_raw_text", () => {
  expect(enumPath("en", "city", { country: 219, region: 3948 })).toBe(
    "/dbenums/country-219/region-3948/city_en.json",
  );
  expect(() => enumPath("../ru", "country", {})).toThrow(/locale/);
  expect(() => enumPath("ru", "region", { country: "219/../secret" })).toThrow(/id/);
});

test("documents_fetch_keeps_the_page_receiver", async () => {
  let receiver: unknown;
  const impl = function (this: unknown) {
    receiver = this;
    return Promise.resolve(new Response(JSON.stringify({ count: 0 }), { status: 200 }));
  };
  const client = new FormClient(impl);
  await expect(client.counts("email", "a@b.c")).resolves.toEqual({ count: 0 });
  expect(receiver).toBe(globalThis);
});

test("documents_failed_counts_hides_the_query", async () => {
  const transport = publicFetch();
  const client = new FormClient(transport.impl);
  transport.failAfter(0);
  await expect(client.counts("email", "a@b.c")).rejects.toThrow("/api/registrations/counts 500");
  await expect(client.search("a@b.c")).rejects.toThrow("/api/registrations 500");
  const counts = client.counts("phone", "+74951234567").then(
    () => "",
    (error: unknown) => (error instanceof Error ? error.message : String(error)),
  );
  const search = client.search("secret person").then(
    () => "",
    (error: unknown) => (error instanceof Error ? error.message : String(error)),
  );
  await expect(counts).resolves.not.toContain("74951234567");
  await expect(search).resolves.not.toContain("secret");
});
