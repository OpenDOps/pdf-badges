import { render, screen, waitFor } from "@testing-library/react";

import { foldForm } from "./form/fold";
import { setUnauthorized } from "./form/client";
import { resetScreenLoads, screenLoads } from "./shell/screens";
import { renderAt } from "./shell/test-router";

function unique(files: readonly string[]) {
  const seen = new Set<string>();
  const ordered: string[] = [];
  for (const file of files) {
    if (seen.has(file)) continue;
    seen.add(file);
    ordered.push(file);
  }
  return ordered;
}

beforeEach(() => {
  resetScreenLoads();
  setUnauthorized(undefined);
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL) => {
      const raw = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      const pathname = new URL(raw, "http://localhost").pathname;
      if (pathname === "/api/sync") {
        return new Response(JSON.stringify({ waiting: 0 }), { status: 200, headers: { "Content-Type": "application/json" } });
      }
      if (pathname === "/forms/form.json") {
        return new Response(JSON.stringify(foldForm()), { status: 200, headers: { "Content-Type": "application/json" } });
      }
      return new Response("[]", { status: 200, headers: { "Content-Type": "application/json" } });
    }),
  );
});

afterEach(() => {
  setUnauthorized(undefined);
  vi.unstubAllGlobals();
});

test("lazy_key_loads_no_other_screen", async () => {
  const { view } = renderAt("/key");
  render(view);
  expect(await screen.findByRole("heading", { name: "Ключ" })).toBeInTheDocument();
  await waitFor(() => expect(screenLoads().length).toBeGreaterThan(0));
  expect(screenLoads().every((file) => file === "routes/key.tsx")).toBe(true);
});

test("lazy_menu_loads_the_tiles", async () => {
  const { view } = renderAt("/");
  render(view);
  expect(await screen.findByRole("link", { name: "Форма регистрации" })).toBeInTheDocument();
  await waitFor(() =>
    expect(unique(screenLoads())).toEqual(["routes/menu.tsx", "routes/desk.tsx", "routes/visitors.tsx"]),
  );
  expect(screenLoads()).not.toContain("routes/import.tsx");
  expect(screenLoads()).not.toContain("routes/settings.tsx");
});

test("lazy_admin_menu_keeps_upload_unloaded", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL) => {
      const raw = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      const pathname = new URL(raw, "http://localhost").pathname;
      if (pathname === "/api/sync") {
        return new Response(JSON.stringify({ waiting: 0, is_admin: true }), {
          status: 200,
          headers: { "Content-Type": "application/json" },
        });
      }
      return new Response("[]", { status: 200, headers: { "Content-Type": "application/json" } });
    }),
  );
  const { view } = renderAt("/");
  render(view);
  expect(await screen.findByRole("button", { name: "Загрузить базу" })).toBeDisabled();
  await waitFor(() =>
    expect(unique(screenLoads())).toEqual([
      "routes/menu.tsx",
      "routes/desk.tsx",
      "routes/visitors.tsx",
      "routes/settings.tsx",
    ]),
  );
  expect(screenLoads()).not.toContain("routes/print.tsx");
  expect(screenLoads()).not.toContain("routes/printers.tsx");
  expect(screenLoads()).not.toContain("routes/registration-settings.tsx");
});

test("lazy_form_loads_the_menu", async () => {
  const { view } = renderAt("/form");
  render(view);
  expect(await screen.findByRole("link", { name: "Главная" })).toBeInTheDocument();
  await waitFor(() =>
    expect(unique(screenLoads())).toEqual(["routes/desk.tsx", "routes/menu.tsx", "routes/visitors.tsx"]),
  );
});

test("lazy_settings_loads_its_two_links", async () => {
  const settings = renderAt("/settings");
  const mounted = render(settings.view);
  expect(await screen.findByRole("link", { name: "Настройки принтеров" })).toBeInTheDocument();
  await waitFor(() => {
    expect(screenLoads()).toContain("routes/printers.tsx");
    expect(screenLoads()).toContain("routes/registration-settings.tsx");
  });
  mounted.unmount();

  resetScreenLoads();
  const printers = renderAt("/printers");
  render(printers.view);
  expect(await screen.findByRole("link", { name: "Форма" })).toHaveAttribute("href", "/form");
  await waitFor(() => expect(screenLoads()).toContain("routes/desk.tsx"));
  expect(screenLoads()).not.toContain("routes/registration-settings.tsx");
});

test("lazy_typed_url_loads_that_screen", async () => {
  const { view } = renderAt("/print");
  render(view);
  expect(await screen.findByRole("link", { name: "Назад" })).toHaveAttribute("href", "/visitors");
  await waitFor(() => {
    expect(screenLoads()).toContain("routes/print.tsx");
    expect(screenLoads()).toContain("routes/visitors.tsx");
  });
  expect(screenLoads()).not.toContain("routes/import.tsx");
});
