import { FormEvent, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

export type LoginError =
  | ""
  | "no_device_id"
  | "invalid_cred"
  | "no_connection"
  | "unknown_error"
  | "session_expired"
  | "login_in_progress";

type LoginProps = {
  onSubmit: (login: string, password: string) => void | Promise<void>;
  error?: LoginError;
};

export default function Login({ onSubmit, error = "" }: LoginProps) {
  const { t } = useTranslation();
  const [login, setLogin] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const inFlight = useRef(false);

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (inFlight.current) {
      return;
    }
    inFlight.current = true;
    setBusy(true);
    try {
      await onSubmit(login, password);
    } finally {
      inFlight.current = false;
      setBusy(false);
    }
  }

  const field =
    "w-full rounded-lg border border-zinc-300 bg-white px-3 py-2 text-zinc-900 outline-none placeholder:text-zinc-400 focus:border-zinc-900 focus:ring-2 focus:ring-zinc-900";

  return (
    <div className="flex min-h-screen items-center justify-center bg-zinc-50 px-4 py-10">
      <form
        onSubmit={handleSubmit}
        aria-label={t("login.title")}
        className="w-full max-w-md rounded-2xl border border-zinc-200 bg-white p-8 shadow-sm"
      >
        <h1 className="text-2xl font-semibold tracking-tight text-zinc-900">
          {t("login.title")}
        </h1>
        <div className="mt-6 space-y-4">
          <input
            type="text"
            name="login"
            placeholder={t("login.email")}
            value={login}
            onChange={(event) => setLogin(event.target.value)}
            autoComplete="username"
            className={field}
          />
          <input
            type="password"
            name="password"
            placeholder={t("login.password")}
            value={password}
            onChange={(event) => setPassword(event.target.value)}
            autoComplete="current-password"
            className={field}
          />
        </div>
        <p role="alert" className="mt-4 min-h-5 text-sm text-red-600">
          {error ? t(`error.${error}`) : ""}
        </p>
        <button
          type="submit"
          disabled={busy}
          className="mt-2 w-full rounded-lg bg-zinc-900 px-4 py-2.5 text-sm font-medium text-white hover:bg-zinc-800 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {t("login.submit")}
        </button>
      </form>
    </div>
  );
}
