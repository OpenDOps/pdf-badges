import { fireEvent, render, screen, waitFor } from "@testing-library/react";

import { foldForm } from "./form/fold";
import { setUnauthorized } from "./form/client";
import { resetScreenLoads } from "./shell/screens";
import { renderAt } from "./shell/test-router";

function stubDesk() {
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL) => {
      const raw = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      const pathname = new URL(raw, "http://localhost").pathname;
      if (pathname === "/api/sync") {
        return new Response(JSON.stringify({ waiting: 0, is_admin: true }), { status: 200, headers: { "Content-Type": "application/json" } });
      }
      if (pathname === "/forms/form.json") {
        return new Response(JSON.stringify(foldForm()), { status: 200, headers: { "Content-Type": "application/json" } });
      }
      return new Response("[]", { status: 200, headers: { "Content-Type": "application/json" } });
    }),
  );
}

beforeEach(() => {
  resetScreenLoads();
  setUnauthorized(undefined);
  stubDesk();
});

afterEach(() => {
  setUnauthorized(undefined);
  vi.unstubAllGlobals();
});

test("formpath_renders_the_desk", async () => {
  const { view } = renderAt("/form");
  render(view);
  expect(await screen.findByRole("button", { name: "Сохранить" })).toBeInTheDocument();
  const form = screen.getByRole("main");
  expect(form).toHaveAttribute("data-screen", "form");
  const status = await screen.findByRole("status");
  expect(status.compareDocumentPosition(form) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(status).not.toHaveClass("fixed");
});

test("formpath_back_opens_the_menu", async () => {
  const { view } = renderAt("/form");
  render(view);
  const home = await screen.findByRole("link", { name: "Главная" });
  expect(home).toHaveAttribute("href", "/");
  expect(screen.queryByRole("button", { name: "Вперед" })).not.toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Форма" })).toHaveAttribute("aria-current", "page");
  expect(screen.getByRole("link", { name: "Посетители" })).toHaveAttribute("href", "/visitors");
  expect(screen.queryByRole("link", { name: "Принтеры" })).not.toBeInTheDocument();
  expect(screen.getByTestId("desk-nav")).toHaveClass("sticky");
  fireEvent.click(home);
  expect(await screen.findByRole("link", { name: "Форма регистрации" })).toHaveAttribute("href", "/form");
  expect(screen.getByRole("link", { name: "Печать" })).toHaveAttribute("href", "/visitors");
  expect(screen.getByRole("button", { name: "Загрузить базу" })).toBeDisabled();
  expect(screen.queryByRole("link", { name: "Загрузить базу" })).not.toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Настройки" })).toHaveAttribute("href", "/settings");
});

test("formpath_desk_redirects", async () => {
  const { router, view } = renderAt("/desk");
  render(view);
  expect(await screen.findByRole("button", { name: "Сохранить" })).toBeInTheDocument();
  await waitFor(() => expect(router.state.location.pathname).toBe("/form"));
  expect(screen.getByRole("main")).toHaveAttribute("data-screen", "form");
});

test("formpath_register_is_unchanged", async () => {
  const { view } = renderAt("/register");
  render(view);
  expect(await screen.findByRole("button", { name: "Начать" })).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Сохранить" })).not.toBeInTheDocument();
});
