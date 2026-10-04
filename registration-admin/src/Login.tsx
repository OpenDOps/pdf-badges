import { FormEvent, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import NetworkSignal, { useNetwork } from "./NetworkSignal";

type LoginProps = {
  onSubmit: (login: string, password: string) => void | Promise<void>;
  error?: string;
  /** Poll `GET /api/network`. Tests that do not mock that call leave this off. */
  watchNetwork?: boolean;
  /** Whole seconds. Used when the network reading is not ready yet. */
  timeoutSecs?: number;
};

export default function Login({
  onSubmit,
  error = "",
  watchNetwork = true,
  timeoutSecs = 10,
}: LoginProps) {
  const { t } = useTranslation();
  const [login, setLogin] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [left, setLeft] = useState(0);
  const network = useNetwork(watchNetwork);
  const inFlight = useRef(false);

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (inFlight.current || left > 0) {
      return;
    }
    const budget = network && network.samples >= 2 ? network.timeoutSecs : timeoutSecs;
    inFlight.current = true;
    setBusy(true);
    setLeft(budget);
    const endsAt = Date.now() + budget * 1000;
    const tick = window.setInterval(() => {
      const remain = Math.max(0, Math.ceil((endsAt - Date.now()) / 1000));
      setLeft(remain);
      if (remain === 0) {
        window.clearInterval(tick);
      }
    }, 1000);
    const hold = new Promise<void>((resolve) => {
      window.setTimeout(resolve, budget * 1000);
    });
    try {
      await onSubmit(login, password);
    } finally {
      await hold;
      window.clearInterval(tick);
      inFlight.current = false;
      setBusy(false);
      setLeft(0);
    }
  }

  const locked = busy || left > 0;

  const field =
    "w-full rounded-lg border border-zinc-300 bg-white px-3 py-2 text-zinc-900 outline-none placeholder:text-zinc-400 focus:border-zinc-900 focus:ring-2 focus:ring-zinc-900";

  return (
    <div className="flex min-h-screen items-center justify-center bg-zinc-50 px-4 py-10">
      <NetworkSignal status={network} />
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
          disabled={locked}
          aria-label={t("login.submit")}
          className="mt-2 w-full rounded-lg bg-zinc-900 px-4 py-2.5 text-sm font-medium text-white hover:bg-zinc-800 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {left > 0 ? t("login.waiting", { count: left }) : t("login.submit")}
        </button>
      </form>
    </div>
  );
}
