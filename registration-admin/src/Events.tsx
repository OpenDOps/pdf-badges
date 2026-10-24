import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { fetchUpdate, postSession, postUpdate } from "./api";
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
  /** Newer release from `GET /api/update`. Absent when this build is current. */
  newer?: string | null;
  /** Version that was rolled back. Absent when the last upgrade was not rolled back. */
  failed?: string | null;
  /** `false` when this process was not started by `rust-reg-run`. */
  supervised?: boolean;
};

export default function Events({
  events,
  current,
  tokenStored,
  onSelect,
  watchNetwork = true,
  keepSession = true,
  timeoutSecs = 10,
  newer = null,
  failed = null,
  supervised = true,
}: EventsProps) {
  const { t } = useTranslation();
  const network = useNetwork(watchNetwork);
  const [busy, setBusy] = useState(false);
  const [left, setLeft] = useState(0);
  const [error, setError] = useState("");
  const inFlight = useRef(false);
  const [upgrade, setUpgrade] = useState<"idle" | "downloading" | "extracting" | "applying" | "success" | "failed">("idle");
  const [received, setReceived] = useState(0);
  const [total, setTotal] = useState<number | null>(null);
  const [target, setTarget] = useState<string | null>(newer);
  const poll = useRef<number | null>(null);
  const posted = useRef(false);

  useEffect(() => {
    setTarget(newer);
  }, [newer]);

  useEffect(() => {
    return () => {
      if (poll.current !== null) {
        window.clearInterval(poll.current);
      }
    };
  }, []);

  useEffect(() => {
    if (upgrade !== "success") {
      return;
    }
    const timer = window.setTimeout(() => setUpgrade("idle"), 10_000);
    return () => window.clearTimeout(timer);
  }, [upgrade]);

  function versionLabel(version: string) {
    return version.startsWith("v") ? version : `v${version}`;
  }

  function stopPoll() {
    if (poll.current !== null) {
      window.clearInterval(poll.current);
      poll.current = null;
    }
  }

  async function watch(wanted: string) {
    try {
      const status = await fetchUpdate();
      if (!status) {
        setUpgrade("applying");
        return;
      }
      if (
        status.version === wanted &&
        status.phase !== "downloading" &&
        status.phase !== "extracting" &&
        status.phase !== "applying"
      ) {
        stopPoll();
        setTarget(null);
        setUpgrade("success");
        return;
      }
      if (status.phase === "downloading" || status.phase === "extracting") {
        setUpgrade(status.phase);
        setReceived(status.received);
        setTotal(status.total);
        return;
      }
      if (posted.current || status.phase === "applying") {
        setUpgrade("applying");
      }
    } catch {
      setUpgrade("applying");
    }
  }

  function beginUpgrade() {
    if (!supervised || !target || upgrade === "downloading" || upgrade === "extracting" || upgrade === "applying") {
      return;
    }
    const wanted = target;
    posted.current = false;
    setUpgrade("downloading");
    setReceived(0);
    setTotal(null);
    stopPoll();
    poll.current = window.setInterval(() => {
      void watch(wanted);
    }, 400);
    void postUpdate()
      .then((result) => {
        if (!result.ok) {
          stopPoll();
          setUpgrade("failed");
          return;
        }
        posted.current = true;
        setUpgrade("applying");
      })
      .catch(() => {
        setUpgrade("applying");
      });
  }

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

  const percent = total && total > 0 ? Math.min(100, Math.round((received / total) * 100)) : null;

  return (
    <main className="min-h-screen bg-zinc-50 px-4 pb-10 pt-16 text-zinc-900">
      <header className="fixed inset-x-0 top-0 z-10 flex h-12 items-center justify-start gap-3 bg-zinc-50 px-4 pr-72">
        {upgrade === "downloading" || upgrade === "extracting" ? (
          <div className="flex items-center gap-2">
            <div className="flex w-56 flex-col gap-1">
              <p className="text-xs text-zinc-600">
                {upgrade === "extracting" ? t("events.extracting") : t("events.downloading")}
              </p>
              <div
                role="progressbar"
                aria-valuemin={0}
                aria-valuemax={total ?? 100}
                aria-valuenow={percent ?? 0}
                className="h-2 overflow-hidden rounded-full bg-zinc-200"
              >
                <div
                  className={percent === null ? "h-full w-1/3 animate-pulse bg-amber-400" : "h-full bg-amber-400"}
                  style={percent === null ? undefined : { width: `${percent}%` }}
                />
              </div>
            </div>
            {percent === null ? null : (
              <span className="w-10 text-right text-xs tabular-nums text-zinc-700">{percent}%</span>
            )}
          </div>
        ) : null}
        {upgrade === "applying" ? (
          <p role="status" className="rounded-full border border-amber-200 bg-amber-50 px-3 py-1 text-sm text-amber-900">
            {t("events.applying")}
          </p>
        ) : null}
        {upgrade === "success" ? (
          <p role="status" className="rounded-full border border-emerald-200 bg-emerald-50 px-3 py-1 text-sm text-emerald-800">
            {t("events.success")}
          </p>
        ) : null}
        {upgrade === "failed" ? (
          <p role="alert" className="text-sm text-red-600">
            {t("events.failed")}
          </p>
        ) : null}
        {failed ? (
          <p role="status" className="text-sm text-red-700">
            {t("events.previous_failed", { version: versionLabel(failed) })}
          </p>
        ) : null}
        {target && upgrade === "idle" ? (
          <button
            type="button"
            disabled={!supervised}
            onClick={beginUpgrade}
            className={
              supervised
                ? "cursor-pointer select-text rounded-full border border-amber-300 bg-amber-50 px-3 py-1 text-sm font-medium text-amber-950"
                : "cursor-not-allowed select-text rounded-full border border-zinc-200 bg-zinc-100 px-3 py-1 text-sm font-medium text-zinc-500"
            }
          >
            {t("events.upgrade", { version: versionLabel(target) })}
          </button>
        ) : null}
        {supervised ? null : (
          <p role="status" className="text-xs text-zinc-600">
            {t("events.no_wrapper")}
          </p>
        )}
      </header>
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
