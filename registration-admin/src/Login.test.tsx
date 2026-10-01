import { act, fireEvent, render, screen } from "@testing-library/react";

import "./i18n";
import Login from "./Login";

function renderLogin(
  props: Partial<{
    onSubmit: (login: string, password: string) => void | Promise<void>;
    error: "" | "invalid_cred";
  }> = {},
) {
  const onSubmit = props.onSubmit ?? (() => {});
  const view = render(<Login onSubmit={onSubmit} error={props.error ?? ""} />);
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

test("login_ignores_submit_while_pending", () => {
  const onSubmit = vi.fn(() => new Promise<void>(() => {}));
  renderLogin({ onSubmit });
  const form = screen.getByRole("form", { name: "Вход в систему" });
  fireEvent.submit(form);
  fireEvent.submit(form);
  expect(onSubmit).toHaveBeenCalledTimes(1);
  expect(screen.getByRole("button", { name: "Войти" })).toBeDisabled();
});
