import i18n from "./i18n";

const CATALOG_KEYS = [
  "login.title",
  "login.email",
  "login.password",
  "login.submit",
  "login.waiting",
  "network.good",
  "network.fair",
  "network.poor",
  "network.offline",
  "network.local",
  "error.bad_request",
  "error.no_device_id",
  "error.invalid_cred",
  "error.no_connection",
  "error.unknown_error",
  "error.session_expired",
  "error.login_in_progress",
  "error.select_in_progress",
  "error.in_progress",
  "events.title",
  "events.choose",
  "events.current",
  "events.empty",
  "events.token_stored",
  "events.token_empty",
  "events.waiting",
  "keys.title",
  "keys.empty",
];

test("ru_has_every_key", () => {
  expect(i18n.language).toBe("ru");
  for (const key of CATALOG_KEYS) {
    const value = i18n.t(key);
    expect(value.length).toBeGreaterThan(0);
    expect(value).not.toBe(key);
  }
  expect(i18n.t("error.invalid_cred")).toBe(
    "Неверные имя пользователя или пароль",
  );
  expect(i18n.t("error.bad_request")).toBe("Некорректный запрос");
  expect(i18n.t("login.submit")).toBe("Войти");
});

test("missing_key_fails", () => {
  expect(() => i18n.t("login.missing")).toThrow(/missing i18n key/);
});
