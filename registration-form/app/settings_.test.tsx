import { fireEvent, render, screen, waitFor } from "@testing-library/react";

import { foldForm } from "./form/fold";
import { setUnauthorized } from "./form/client";
import { renderAt } from "./shell/test-router";

beforeEach(() => {
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
      if (pathname === "/api/desk/catalog") {
        return new Response(
          JSON.stringify({
            rev: 1,
            categories: [],
            printers: [{ id: "ipp://printer", name: "HP", text: "HP" }],
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      }
      return new Response("[]", { status: 200, headers: { "Content-Type": "application/json" } });
    }),
  );
});

afterEach(() => {
  setUnauthorized(undefined);
  vi.unstubAllGlobals();
});

test("settings_uses_the_desk_header_and_two_links", async () => {
  const { router, view } = renderAt("/settings");
  render(view);
  expect(await screen.findByRole("main")).toHaveAttribute("data-screen", "settings");
  expect(screen.getByTestId("desk-nav")).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Главная" })).toHaveAttribute("href", "/");
  expect(screen.getByRole("link", { name: "Форма" })).toHaveAttribute("href", "/form");
  expect(screen.getByRole("link", { name: "Посетители" })).toHaveAttribute("href", "/visitors");
  expect(screen.getByLabelText("Принтер")).toBeInTheDocument();
  const printerButton = screen.getByRole("button", { name: "Настроить принтеры" });
  expect(printerButton.querySelector("svg")).toBeTruthy();
  expect(printerButton.closest("[data-testid='desk-nav']")).toBeTruthy();
  expect(screen.queryByRole("link", { name: "Настройки регистрации" })).not.toBeInTheDocument();
  expect(screen.queryByText("Перезапуск системы")).not.toBeInTheDocument();
  expect(screen.queryByRole("link", { name: "Назад" })).not.toBeInTheDocument();
  expect(screen.queryByText("Установить текущее время на роутере")).not.toBeInTheDocument();
  expect(document.querySelector("[data-version]")).toBeNull();
  fireEvent.click(printerButton);
  await waitFor(() => expect(router.state.location.pathname).toBe("/printers"));
  const selected = screen.getByRole("button", { name: "Настроить принтеры" });
  expect(selected).toHaveAttribute("aria-current", "page");
  expect(selected.className).toContain("bg-[#1d4ed8]");
  expect(selected.querySelector("svg")).toBeTruthy();
  expect(screen.queryByLabelText("Принтер")).not.toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Форма" })).not.toHaveAttribute("aria-current");
  expect(screen.getByRole("link", { name: "Посетители" })).not.toHaveAttribute("aria-current");
});
