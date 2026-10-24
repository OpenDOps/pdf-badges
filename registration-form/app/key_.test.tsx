import { fireEvent, render, screen, waitFor } from "@testing-library/react";

import { setUnauthorized } from "./form/client";
import { resetScreenLoads, screenLoads } from "./shell/screens";
import { renderAt } from "./shell/test-router";

function authPosts(fetchMock: ReturnType<typeof vi.fn>) {
  return fetchMock.mock.calls.filter((call) => {
    const input = call[0] as RequestInfo | URL;
    const raw = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    return new URL(raw, "http://localhost").pathname === "/api/desk/auth";
  });
}

function stubAuth(status: number, event: { id: string | null; name: string | null } = { id: null, name: null }) {
  const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
    const raw = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
    const pathname = new URL(raw, "http://localhost").pathname;
    if (pathname === "/api/desk/auth") {
      const body = status === 200 ? {} : { error: { code: "invalid_key" } };
      return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });
    }
    if (pathname === "/api/event") {
      return new Response(JSON.stringify({ ok: true, data: event }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      });
    }
    if (pathname === "/api/sync") {
      return new Response(JSON.stringify({ waiting: 0 }), { status: 200, headers: { "Content-Type": "application/json" } });
    }
    return new Response("[]", { status: 200, headers: { "Content-Type": "application/json" } });
  });
  vi.stubGlobal("fetch", fetchMock);
  return fetchMock;
}

beforeEach(() => {
  resetScreenLoads();
  setUnauthorized(undefined);
});

afterEach(() => {
  setUnauthorized(undefined);
  vi.unstubAllGlobals();
});

test("key_shows_the_field", async () => {
  stubAuth(200);
  const { view } = renderAt("/key");
  render(view);
  expect(await screen.findByPlaceholderText("Ключ")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Войти" })).toBeInTheDocument();
  expect(screen.queryByRole("heading", { name: "Ключ" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Назад" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Вперед" })).not.toBeInTheDocument();
  expect(screen.queryByRole("link", { name: "Назад" })).not.toBeInTheDocument();
});

test("key_empty_does_not_post", async () => {
  const fetchMock = stubAuth(200);
  const { router, view } = renderAt("/key");
  render(view);
  fireEvent.click(await screen.findByRole("button", { name: "Войти" }));
  expect(router.state.location.pathname).toBe("/key");
  expect(authPosts(fetchMock)).toHaveLength(0);
});

test("key_unknown_stays", async () => {
  stubAuth(401);
  const { router, view } = renderAt("/key");
  render(view);
  const field = await screen.findByPlaceholderText("Ключ");
  fireEvent.change(field, { target: { value: "abc" } });
  fireEvent.click(screen.getByRole("button", { name: "Войти" }));
  expect(await screen.findByText("Неверный ключ")).toBeInTheDocument();
  expect(router.state.location.pathname).toBe("/key");
  expect(field).toHaveValue("abc");
});

test("key_accepted_opens_the_menu", async () => {
  const fetchMock = stubAuth(200);
  const { router, view } = renderAt("/key");
  render(view);
  const field = await screen.findByPlaceholderText("Ключ");
  expect(screenLoads()).not.toContain("routes/menu.tsx");
  fireEvent.change(field, { target: { value: "desk" } });
  fireEvent.click(screen.getByRole("button", { name: "Войти" }));
  await waitFor(() => expect(screen.getByRole("main")).toHaveAttribute("data-screen", "menu"));
  expect(router.state.location.pathname).toBe("/");
  expect(screenLoads()).toContain("routes/menu.tsx");
  const init = authPosts(fetchMock)[0]?.[1] as RequestInit | undefined;
  expect(JSON.parse(String(init?.body))).toEqual({ key: "desk" });
});

test("key_shows_the_selected_event", async () => {
  stubAuth(200, { id: "GELCPAQSFL", name: "Выставка" });
  const { view } = renderAt("/key");
  render(view);
  expect(await screen.findByText("Выставка")).toBeInTheDocument();
  expect(screen.getByText("UID GELCPAQSFL")).toBeInTheDocument();
  expect(screen.queryByText("Мероприятие не выбрано")).not.toBeInTheDocument();
  expect(screen.queryByRole("link", { name: "Выбрать мероприятие" })).not.toBeInTheDocument();
});

test("key_points_at_admin_when_no_event_is_selected", async () => {
  stubAuth(200);
  const { view } = renderAt("/key");
  render(view);
  expect(await screen.findByText("Мероприятие не выбрано")).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Выбрать мероприятие" })).toHaveAttribute("href", "/admin");
  expect(screen.queryByText(/^UID /)).not.toBeInTheDocument();
});

test("key_has_no_key_admin", async () => {
  stubAuth(200, { id: "GELCPAQSFL", name: "Выставка" });
  const { view } = renderAt("/key");
  render(view);
  expect(await screen.findByRole("button", { name: "Войти" })).toBeInTheDocument();
  expect(screen.queryByRole("link")).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Сохранить" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /печат/i })).not.toBeInTheDocument();
});
