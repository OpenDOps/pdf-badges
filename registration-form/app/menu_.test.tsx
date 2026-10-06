import { render, screen } from "@testing-library/react";

import { setUnauthorized } from "./form/client";
import { resetScreenLoads } from "./shell/screens";
import { renderAt } from "./shell/test-router";

function stubSync(waiting: number, admin = false) {
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL) => {
      const raw = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      const pathname = new URL(raw, "http://localhost").pathname;
      if (pathname === "/api/sync") {
        return new Response(JSON.stringify({ waiting, is_admin: admin }), { status: 200, headers: { "Content-Type": "application/json" } });
      }
      return new Response("[]", { status: 200, headers: { "Content-Type": "application/json" } });
    }),
  );
}

beforeEach(() => {
  resetScreenLoads();
  setUnauthorized(undefined);
  stubSync(3);
});

afterEach(() => {
  setUnauthorized(undefined);
  vi.unstubAllGlobals();
});

test("menu_admin_shows_four_tiles", async () => {
  stubSync(3, true);
  const { view } = renderAt("/");
  render(view);
  const links = await screen.findAllByRole("link");
  expect(links.map((link) => [link.textContent, link.getAttribute("href")])).toEqual([
    ["Форма регистрации", "/form"],
    ["Печать", "/visitors"],
    ["Настройки", "/settings"],
  ]);
  const upload = screen.getByRole("button", { name: "Загрузить базу" });
  expect(upload).toBeDisabled();
  expect(screen.queryByRole("link", { name: "Загрузить базу" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Назад" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Вперед" })).not.toBeInTheDocument();
  expect(screen.queryByRole("link", { name: "Назад" })).not.toBeInTheDocument();
});

test("menu_operator_shows_form_and_visitors", async () => {
  stubSync(0, false);
  const { view } = renderAt("/");
  render(view);
  const links = await screen.findAllByRole("link");
  expect(links.map((link) => [link.textContent, link.getAttribute("href")])).toEqual([
    ["Форма регистрации", "/form"],
    ["Печать", "/visitors"],
  ]);
  expect(screen.queryByRole("button", { name: "Загрузить базу" })).not.toBeInTheDocument();
  expect(screen.queryByRole("link", { name: "Настройки" })).not.toBeInTheDocument();
});

test("menu_tiles_follow_the_pointer", async () => {
  stubSync(3, true);
  const { view } = renderAt("/");
  render(view);
  const names = ["Форма регистрации", "Печать", "Настройки"];
  for (const name of names) {
    const tile = (await screen.findByRole("link", { name })).parentElement;
    expect(tile?.className).toContain("radial-gradient");
  }
  const upload = screen.getByRole("button", { name: "Загрузить базу" }).parentElement;
  expect(upload?.className).toContain("radial-gradient");
  const form = screen.getByRole("link", { name: "Форма регистрации" }).parentElement;
  if (!form || !upload) throw new Error("tile");
  form.dispatchEvent(new MouseEvent("mousemove", { clientX: 48, clientY: 30, bubbles: true }));
  expect(form.style.getPropertyValue("--spot-x")).toBe("48px");
  expect(form.style.getPropertyValue("--spot-y")).toBe("30px");
  upload.dispatchEvent(new MouseEvent("mousemove", { clientX: 12, clientY: 80, bubbles: true }));
  expect(upload.style.getPropertyValue("--spot-x")).toBe("12px");
  expect(upload.style.getPropertyValue("--spot-y")).toBe("80px");
});

test("menu_shows_the_waiting_count", async () => {
  const { view } = renderAt("/");
  render(view);
  expect(await screen.findByText("3")).toBeInTheDocument();
});

test("menu_shows_the_register_qr", async () => {
  const { view } = renderAt("/");
  render(view);
  const img = await screen.findByRole("img");
  const src = img.getAttribute("src") ?? "";
  expect(src).toContain("/api/qr");
  expect(src).toContain(encodeURIComponent("/register"));
});

test("menu_shows_sync_address_and_network", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL) => {
      const raw = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      const pathname = new URL(raw, "http://localhost").pathname;
      if (pathname === "/api/sync") {
        return new Response(
          JSON.stringify({ ok: true, data: { waiting: 2, synced: 5, addresses: ["10.0.0.8"] } }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      }
      if (pathname === "/api/network") {
        return new Response(
          JSON.stringify({ ok: true, data: { samples: 2, offline: false, quality: "good", timeout_secs: 5 } }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      }
      return new Response("[]", { status: 200, headers: { "Content-Type": "application/json" } });
    }),
  );
  const { view } = renderAt("/");
  render(view);
  expect(await screen.findByText("2")).toBeInTheDocument();
  expect(screen.queryByText("5")).not.toBeInTheDocument();
  expect(screen.getByText("10.0.0.8")).toBeInTheDocument();
  expect(screen.getByText("Не синхронизировано")).toBeInTheDocument();
  expect(screen.queryByText("Синхронизировано")).not.toBeInTheDocument();
  expect(screen.queryByRole("heading", { name: "Главная" })).not.toBeInTheDocument();
  const remote = await screen.findByText("Сервер синхронизации доступен");
  expect(screen.queryByText("Локальный сервер доступен")).not.toBeInTheDocument();
  expect(remote.closest("[data-signal]")).toHaveAttribute("data-signal", "good");
  const quality = screen.getByText("Связь хорошая");
  expect(quality.closest("[data-signal]")).toHaveAttribute("data-signal", "good");
});

test("menu_hides_unmeasured_quality_when_remote_is_down", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL) => {
      const raw = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      const pathname = new URL(raw, "http://localhost").pathname;
      if (pathname === "/api/sync") {
        return new Response(JSON.stringify({ waiting: 0 }), { status: 200, headers: { "Content-Type": "application/json" } });
      }
      if (pathname === "/api/network") {
        return new Response(
          JSON.stringify({ ok: true, data: { samples: 2, offline: true, quality: null, timeout_secs: 2 } }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      }
      return new Response("[]", { status: 200, headers: { "Content-Type": "application/json" } });
    }),
  );
  const { view } = renderAt("/");
  render(view);
  expect(await screen.findByText("Нет соединения с сервером синхронизации")).toBeInTheDocument();
  expect(screen.queryByText("Качество связи ещё не измерено")).not.toBeInTheDocument();
});

test("menu_shows_local_down_in_red", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL) => {
      const raw = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      const pathname = new URL(raw, "http://localhost").pathname;
      if (pathname === "/api/sync") {
        return new Response(JSON.stringify({ waiting: 0 }), { status: 200, headers: { "Content-Type": "application/json" } });
      }
      if (pathname === "/api/network") {
        return new Response("", { status: 500 });
      }
      return new Response("[]", { status: 200, headers: { "Content-Type": "application/json" } });
    }),
  );
  const { view } = renderAt("/");
  render(view);
  const local = await screen.findByText("Нет соединения с локальным сервером");
  expect(local.closest("[data-signal]")).toHaveAttribute("data-signal", "bad");
  expect(screen.queryByText("Локальный сервер доступен")).not.toBeInTheDocument();
});

test("menu_omits_update_and_moderation", async () => {
  const { view } = renderAt("/");
  render(view);
  expect(await screen.findByRole("link", { name: "Форма регистрации" })).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Обновить" })).not.toBeInTheDocument();
  expect(screen.queryByRole("checkbox", { name: "Включить модерацию" })).not.toBeInTheDocument();
});
