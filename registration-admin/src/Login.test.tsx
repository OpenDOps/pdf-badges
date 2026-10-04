import { useState } from "react";
import { act, fireEvent, render, screen } from "@testing-library/react";

import { postLogin } from "./api";
import "./i18n";
import Login from "./Login";

function renderLogin(
  props: Partial<{
    onSubmit: (login: string, password: string) => void | Promise<void>;
    error: "" | "invalid_cred";
  }> = {},
) {
  const onSubmit = props.onSubmit ?? (() => {});
  const view = render(
    <Login
      onSubmit={onSubmit}
      error={props.error ?? ""}
      watchNetwork={false}
      timeoutSecs={0}
    />,
  );
  return { ...view, onSubmit };
}

test("login_shows_russian", () => {
  renderLogin();
  expect(
    screen.getByRole("heading", { name: "Вход в систему" }),
  ).toBeInTheDocument();
  expect(screen.getByPlaceholderText("Введите свой email")).toBeInTheDocument();
  expect(
    screen.getByPlaceholderText("Введите свой пароль"),
  ).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Войти" })).toBeInTheDocument();
});

test("login_submits_fields", async () => {
  const onSubmit = vi.fn();
  renderLogin({ onSubmit });
  fireEvent.change(screen.getByPlaceholderText("Введите свой email"), {
    target: { value: "ann@example.com" },
  });
  fireEvent.change(screen.getByPlaceholderText("Введите свой пароль"), {
    target: { value: "secret" },
  });
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Войти" }));
  });
  expect(onSubmit).toHaveBeenCalledTimes(1);
  expect(onSubmit).toHaveBeenCalledWith("ann@example.com", "secret");
});

test("login_shows_invalid_cred", () => {
  renderLogin({ error: "invalid_cred" });
  expect(screen.getByRole("alert")).toHaveTextContent(
    "Неверные имя пользователя или пароль",
  );
});

test("login_hides_error_when_empty", () => {
  renderLogin({ error: "" });
  expect(screen.getByRole("alert")).toHaveTextContent("");
});

test("login_screen_shows_server_error", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue({
      json: async () => ({ ok: false, error: { code: "invalid_cred" } }),
    }),
  );
  function Harness() {
    const [error, setError] = useState("");
    return (
      <Login
        error={error}
        watchNetwork={false}
        timeoutSecs={0}
        onSubmit={async (login, password) => {
          const result = await postLogin(login, password);
          if (!result.ok) {
            setError(result.code);
          }
        }}
      />
    );
  }
  render(<Harness />);
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Войти" }));
  });
  expect(screen.getByRole("alert")).toHaveTextContent(
    "Неверные имя пользователя или пароль",
  );
  vi.unstubAllGlobals();
});

test("login_shows_quality_after_two_samples", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue({
      json: async () => ({
        ok: true,
        data: { samples: 2, offline: false, quality: "fair", timeout_secs: 6 },
      }),
    }),
  );
  render(<Login onSubmit={() => {}} watchNetwork />);
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
  const signal = await screen.findByRole("status");
  expect(signal).toHaveTextContent("Связь средняя");
  expect(signal).toHaveAttribute("data-signal", "warn");
  expect(signal).toHaveClass("fixed", "top-4", "right-4");
  vi.unstubAllGlobals();
});

test("login_shows_good_connection_in_green", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue({
      json: async () => ({
        ok: true,
        data: { samples: 2, offline: false, quality: "good", timeout_secs: 3 },
      }),
    }),
  );
  render(<Login onSubmit={() => {}} watchNetwork />);
  const signal = await screen.findByRole("status");
  expect(signal).toHaveTextContent("Связь хорошая");
  expect(signal).toHaveAttribute("data-signal", "good");
  vi.unstubAllGlobals();
});

test("login_shows_poor_connection_in_red", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue({
      json: async () => ({
        ok: true,
        data: { samples: 2, offline: false, quality: "poor", timeout_secs: 10 },
      }),
    }),
  );
  render(<Login onSubmit={() => {}} watchNetwork />);
  const signal = await screen.findByRole("status");
  expect(signal).toHaveTextContent("Связь слабая");
  expect(signal).toHaveAttribute("data-signal", "bad");
  vi.unstubAllGlobals();
});

test("login_hides_quality_before_two_samples", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue({
      json: async () => ({
        ok: true,
        data: { samples: 1, offline: false, quality: null, timeout_secs: 10 },
      }),
    }),
  );
  render(<Login onSubmit={() => {}} watchNetwork />);
  await act(async () => {
    await Promise.resolve();
  });
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
  vi.unstubAllGlobals();
});

test("login_shows_offline", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue({
      json: async () => ({
        ok: true,
        data: { samples: 2, offline: true, quality: null, timeout_secs: 2 },
      }),
    }),
  );
  render(<Login onSubmit={() => {}} watchNetwork />);
  const signal = await screen.findByRole("status");
  expect(signal).toHaveTextContent("Нет соединения с интернет-сервером");
  expect(signal).toHaveAttribute("data-signal", "bad");
  vi.unstubAllGlobals();
});

test("login_shows_local_server_down", async () => {
  vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new Error("failed")));
  render(<Login onSubmit={() => {}} watchNetwork />);
  const signal = await screen.findByRole("status");
  expect(signal).toHaveTextContent(
    "Нет соединения с локальным сервером. Вероятно, он выключен",
  );
  expect(signal).toHaveAttribute("data-signal", "bad");
  vi.unstubAllGlobals();
});

test("login_counts_down_for_the_timeout", async () => {
  vi.useFakeTimers();
  const onSubmit = vi.fn().mockResolvedValue(undefined);
  render(<Login onSubmit={onSubmit} watchNetwork={false} timeoutSecs={3} />);
  const button = screen.getByRole("button", { name: "Войти" });
  act(() => {
    fireEvent.click(button);
  });
  expect(button).toHaveTextContent("Подождите 3 с");
  expect(button).toBeDisabled();
  await act(async () => {
    await vi.advanceTimersByTimeAsync(1000);
  });
  expect(button).toHaveTextContent("Подождите 2 с");
  await act(async () => {
    await vi.advanceTimersByTimeAsync(2000);
  });
  expect(button).toHaveTextContent("Войти");
  expect(button).toBeEnabled();
  expect(onSubmit).toHaveBeenCalledTimes(1);
  vi.useRealTimers();
});

test("login_ignores_submit_while_pending", () => {
  const onSubmit = vi.fn(() => new Promise<void>(() => {}));
  renderLogin({ onSubmit });
  const form = screen.getByRole("form", { name: "Вход в систему" });
  fireEvent.submit(form);
  fireEvent.submit(form);
  expect(onSubmit).toHaveBeenCalledTimes(1);
  expect(screen.getByRole("button", { name: "Войти" })).toBeDisabled();
});
