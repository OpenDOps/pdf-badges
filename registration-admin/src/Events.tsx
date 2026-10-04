import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { postSession } from "./api";
import NetworkSignal, { useNetwork } from "./NetworkSignal";

const SESSION_PING_MS = 30_000;

export type Event = {
  id: string;
  name: string;
};

type EventsProps = {
  events: Event[];
  current: Event | null;
  tokenStored: boolean;
  onSelect: (id: string, name: string) => void | Promise<void | string>;
  /** Poll `GET /api/network`. Tests that do not mock that call leave this off. */
  watchNetwork?: boolean;
  /** `POST /api/session` every 30s so the one-minute operator lease stays open. */
  keepSession?: boolean;
  /** Whole seconds. Used when the network reading is not ready yet. */
  timeoutSecs?: number;
};

export default function Events({
  events,
  current,
  tokenStored,
  onSelect,
  watchNetwork = true,
  keepSession = true,
  timeoutSecs = 10,
}: EventsProps) {
  const { t } = useTranslation();
  const network = useNetwork(watchNetwork);
  const [busy, setBusy] = useState(false);
  const [left, setLeft] = useState(0);
  const [error, setError] = useState("");
  const inFlight = useRef(false);

  useEffect(() => {
    if (!keepSession) {
      return;
    }
    void postSession().catch(() => {});
    const timer = window.setInterval(() => {
      void postSession().catch(() => {});
    }, SESSION_PING_MS);
    return () => window.clearInterval(timer);
  }, [keepSession]);

  async function choose(id: string, name: string) {
    if (inFlight.current || left > 0) {
      return;
    }
    const budget = network && network.samples >= 2 ? network.timeoutSecs : timeoutSecs;
    inFlight.current = true;
    setBusy(true);
    setError("");
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
      const result = await onSelect(id, name);
      if (typeof result === "string" && result) {
        setError(result);
      }
    } finally {
      await hold;
      window.clearInterval(tick);
      inFlight.current = false;
      setBusy(false);
      setLeft(0);
    }
  }

  const locked = busy || left > 0;

  return (
    <main className="min-h-screen bg-zinc-50 px-4 py-10 text-zinc-900">
      <NetworkSignal status={network} />
      {left > 0 ? (
        <div className="fixed inset-0 z-20 flex items-center justify-center bg-zinc-900/40 px-4">
          <div
            role="progressbar"
            aria-valuemin={0}
            aria-valuemax={network && network.samples >= 2 ? network.timeoutSecs : timeoutSecs}
            aria-valuenow={left}
            className="flex w-full max-w-xs flex-col items-center gap-3 rounded-2xl bg-white px-8 py-6 text-center shadow-lg"
          >
            <span
              aria-hidden="true"
              className="h-8 w-8 animate-spin rounded-full border-2 border-zinc-200 border-t-zinc-900"
            />
            <p>{t("events.waiting", { count: left })}</p>
            {error ? (
              <p role="alert" className="text-sm text-red-600">
                {t(`error.${error}`)}
              </p>
            ) : null}
          </div>
        </div>
      ) : null}
      <div className="mx-auto flex w-full max-w-lg flex-col gap-6">
        <h1 className="text-2xl font-semibold tracking-tight">
          {t("events.title")}
        </h1>
        <section
          aria-labelledby="current-event"
          className="rounded-2xl border border-zinc-200 bg-white p-5 shadow-sm"
        >
          <h2
            id="current-event"
            className="text-sm font-medium text-zinc-500"
          >
            {t("events.current")}
          </h2>
          <p className="mt-1 text-lg font-medium">
            {current?.name ?? ""}
          </p>
        </section>
        <section aria-labelledby="choose-event">
          <h2 id="choose-event" className="text-sm font-medium text-zinc-500">
            {t("events.choose")}
          </h2>
          {events.length === 0 ? (
            <p className="mt-3 rounded-2xl border border-dashed border-zinc-300 bg-white px-4 py-8 text-center text-sm text-zinc-500">
              {t("events.empty")}
            </p>
          ) : (
            <ul className="mt-3 flex flex-col gap-2">
              {events.map((event) => {
                const selected = event.id === current?.id;
                return (
                  <li key={event.id}>
                    <button
                      type="button"
                      disabled={locked}
                      onClick={() => void choose(event.id, event.name)}
                      className={
                        selected
                          ? "w-full rounded-xl border border-zinc-900 bg-zinc-900 px-4 py-3 text-left text-sm font-medium text-white disabled:cursor-not-allowed disabled:opacity-50"
                          : "w-full rounded-xl border border-zinc-200 bg-white px-4 py-3 text-left text-sm font-medium hover:border-zinc-400 disabled:cursor-not-allowed disabled:opacity-50"
                      }
                    >
                      {event.name}
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </section>
        {left === 0 && error ? (
          <p role="alert" className="text-sm text-red-600">
            {t(`error.${error}`)}
          </p>
        ) : null}
        <p
          role="status"
          className={
            tokenStored
              ? "rounded-xl bg-emerald-50 px-4 py-3 text-sm text-emerald-800"
              : "rounded-xl bg-zinc-100 px-4 py-3 text-sm text-zinc-600"
          }
        >
          {tokenStored
            ? t("events.token_stored")
            : t("events.token_empty")}
        </p>
        <section
          aria-labelledby="keys-title"
          className="rounded-2xl border border-dashed border-zinc-300 bg-white p-5"
        >
          <h2 id="keys-title" className="text-sm font-medium text-zinc-500">
            {t("keys.title")}
          </h2>
          <p className="mt-1 text-sm text-zinc-500">{t("keys.empty")}</p>
        </section>
      </div>
    </main>
  );
}
