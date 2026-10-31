import { fireEvent, render, screen, waitFor } from "@testing-library/react";

import { foldForm } from "./form/fold";
import { setUnauthorized } from "./form/client";
import { renderAt } from "./shell/test-router";

type Call = { method: string; pathname: string; body: unknown };

function callsOf(fetchMock: ReturnType<typeof vi.fn>): Call[] {
  return fetchMock.mock.calls.map((call) => {
    const [input, init] = call as [RequestInfo | URL, RequestInit | undefined];
    const raw = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    const method = init?.method ?? "GET";
    let body: unknown = null;
    if (typeof init?.body === "string") body = JSON.parse(init.body);
    return { method, pathname: new URL(raw, "http://localhost").pathname, body };
  });
}

function stub(settings: Record<string, boolean>, pools: unknown[]) {
  const fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const raw = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    const pathname = new URL(raw, "http://localhost").pathname;
    const method = init?.method ?? "GET";
    if (pathname === "/api/sync" && method === "GET") {
      return new Response(JSON.stringify({ waiting: 0, is_admin: true }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      });
    }
    if (pathname === "/api/settings" && method === "GET") {
      return new Response(JSON.stringify({ ok: true, data: settings }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      });
    }
    if (pathname === "/api/settings" && method === "PATCH") {
      return new Response(JSON.stringify({ ok: true, data: { ...settings, ...JSON.parse(String(init?.body)) } }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      });
    }
    if (pathname === "/api/barcodes" && method === "GET") {
      return new Response(JSON.stringify({ ok: true, data: pools }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      });
    }
    if (pathname === "/api/barcodes" && method === "PATCH") {
      return new Response(JSON.stringify({ ok: true, data: pools }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      });
    }
    if (pathname === "/api/sync" && method === "POST") {
      return new Response(JSON.stringify({ ok: true, data: {} }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      });
    }
    if (pathname === "/forms/form.json") {
      return new Response(JSON.stringify(foldForm()), { status: 200, headers: { "Content-Type": "application/json" } });
    }
    if (pathname === "/api/desk/catalog") {
      return new Response(JSON.stringify({ rev: 1, categories: [], printers: [] }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      });
    }
    return new Response("[]", { status: 200, headers: { "Content-Type": "application/json" } });
  });
  vi.stubGlobal("fetch", fetchMock);
  return fetchMock;
}

beforeEach(() => {
  setUnauthorized(undefined);
});

afterEach(() => {
  setUnauthorized(undefined);
  vi.unstubAllGlobals();
});

test("regset_loads_the_switches", async () => {
  stub({ double_print: true, load_barcodes: false }, []);
  const { view } = renderAt("/settings");
  render(view);
  await waitFor(() => expect(screen.getByRole("checkbox", { name: "Печатать второй бейдж" })).toBeChecked());
  expect(screen.getByRole("checkbox", { name: "Загружать штрихкоды" })).not.toBeChecked();
});

test("regset_patch_is_one_field", async () => {
  const fetchMock = stub({ capitalize_names: false }, []);
  const { view } = renderAt("/settings");
  render(view);
  fireEvent.click(await screen.findByRole("checkbox", { name: "Делать ФИО с заглавной буквы" }));
  await waitFor(() => {
    const patch = callsOf(fetchMock).find((call) => call.pathname === "/api/settings" && call.method === "PATCH");
    expect(patch?.body).toEqual({ capitalize_names: true });
  });
});

test("regset_barcode_min_and_add", async () => {
  const pools = [{ cat_id: 3, name: "Гость", remaining: 4, total: 20, min: 1, add: 2 }];
  const fetchMock = stub({}, pools);
  const { view } = renderAt("/settings");
  render(view);
  const row = await screen.findByRole("button", { name: /Гость/ });
  expect(row).toHaveTextContent("4 / 20");
  fireEvent.click(row);
  fireEvent.change(screen.getByLabelText("Минимум"), { target: { value: "5" } });
  fireEvent.change(screen.getByLabelText("Добавить"), { target: { value: "10" } });
  fireEvent.click(screen.getByRole("button", { name: "отмена" }));
  expect(callsOf(fetchMock).some((call) => call.pathname === "/api/barcodes" && call.method === "PATCH")).toBe(false);

  fireEvent.click(screen.getByRole("button", { name: /Гость/ }));
  fireEvent.change(screen.getByLabelText("Минимум"), { target: { value: "5" } });
  fireEvent.change(screen.getByLabelText("Добавить"), { target: { value: "10" } });
  fireEvent.click(screen.getByRole("button", { name: "ок" }));
  await waitFor(() => {
    const patch = callsOf(fetchMock).find((call) => call.pathname === "/api/barcodes" && call.method === "PATCH");
    expect(patch?.body).toEqual({ cat_id: 3, min: 5, add: 10 });
  });
});

test("regset_restarts_sync", async () => {
  const fetchMock = stub({}, []);
  const { view } = renderAt("/settings");
  render(view);
  fireEvent.click(await screen.findByRole("button", { name: "Перезапуск синхронизации" }));
  await waitFor(() => {
    expect(callsOf(fetchMock).some((call) => call.pathname === "/api/sync" && call.method === "POST")).toBe(true);
  });
  expect(screen.queryByText("Перезапуск системы")).not.toBeInTheDocument();
  expect(screen.queryByRole("link", { name: "Настройки регистрации" })).not.toBeInTheDocument();
});
