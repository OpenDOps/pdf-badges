import { fireEvent, render, screen } from "@testing-library/react";

import App from "./App";
import "./i18n";

beforeEach(() => {
  window.history.pushState({}, "", "/");
});

afterEach(() => {
  vi.unstubAllGlobals();
});

function json(body: unknown) {
  return { json: async () => body };
}

function mockApi() {
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (url.endsWith("/api/network")) {
        return json({
          ok: true,
          data: {
            samples: 2,
            offline: false,
            quality: "good",
            timeout_secs: 0,
          },
        });
      }
      if (url.endsWith("/api/login") && init?.method === "POST") {
        return json({ ok: true, data: { timeout_secs: 0 } });
      }
      if (url.endsWith("/api/events") && init?.method !== "POST") {
        return json({
          ok: true,
          data: {
            current: null,
            events: [{ id: "5245081", name: "TechCrunch" }],
          },
        });
      }
      if (url.endsWith("/api/binding")) {
        return json({
          ok: true,
          data: { id: null, name: null, token_stored: false },
        });
      }
      if (url.endsWith("/api/session")) {
        return json({ ok: true });
      }
      if (url.endsWith("/api/events/select")) {
        return json({
          ok: true,
          data: { id: "5245081", name: "TechCrunch", token_stored: true },
        });
      }
      throw new Error(url);
    }),
  );
}

test("app_shows_login", () => {
  render(<App />);
  expect(
    screen.getByRole("heading", { name: "Вход в систему" }),
  ).toBeInTheDocument();
});

test("app_shows_events", async () => {
  mockApi();
  window.history.pushState({}, "", "/events");
  render(<App />);
  expect(
    await screen.findByRole("heading", { name: "Мероприятия" }),
  ).toBeInTheDocument();
  expect(
    await screen.findByRole("button", { name: "TechCrunch" }),
  ).toBeInTheDocument();
});

test("app_opens_events_after_login", async () => {
  mockApi();
  render(<App />);
  await screen.findByRole("status", { name: "Связь хорошая" });
  fireEvent.change(screen.getByPlaceholderText("Введите свой email"), {
    target: { value: "a@b.c" },
  });
  fireEvent.change(screen.getByPlaceholderText("Введите свой пароль"), {
    target: { value: "secret" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Войти" }));
  expect(
    await screen.findByRole("heading", { name: "Мероприятия" }),
  ).toBeInTheDocument();
  expect(window.location.pathname).toBe("/events");
});
