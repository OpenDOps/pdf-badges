import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { FormClient } from "../form/client";

type Quality = "good" | "fair" | "poor" | null;

type Reading = {
  samples: number;
  offline: boolean;
  quality: Quality;
};

type Corner = {
  local: boolean;
  remote: "up" | "down" | "unknown";
  quality: Quality | "unknown";
};

function cornerOf(body: unknown): Corner | null {
  if (!body || typeof body !== "object" || !("samples" in body)) return null;
  const reading = body as Reading;
  const ready = typeof reading.samples === "number" && reading.samples >= 2;
  return {
    local: true,
    remote: !ready ? "unknown" : reading.offline ? "down" : "up",
    quality: !ready || reading.offline ? "unknown" : reading.quality,
  };
}

export function NetworkCorner({ pinned = true }: { pinned?: boolean }) {
  const { t } = useTranslation();
  const [corner, setCorner] = useState<Corner | null>(null);

  useEffect(() => {
    let cancel = false;
    async function load() {
      try {
        const body = await new FormClient().network();
        if (cancel) return;
        setCorner(cornerOf(body) ?? { local: false, remote: "unknown", quality: "unknown" });
      } catch {
        if (!cancel) setCorner({ local: false, remote: "unknown", quality: "unknown" });
      }
    }
    void load();
    const timer = window.setInterval(() => void load(), 5000);
    return () => {
      cancel = true;
      window.clearInterval(timer);
    };
  }, []);

  const line = tickerOf(corner, t);
  if (!line) return null;

  return (
    <p role="status" data-network data-pinned={pinned ? "" : undefined} className="flex min-w-0 flex-row items-center">
      <StatusLine signal={line.signal} text={line.text} />
    </p>
  );
}

function tickerOf(
  corner: Corner | null,
  t: (key: string) => string,
): { signal: Signal; text: string } | null {
  if (!corner) return null;
  if (!corner.local) return { signal: "bad", text: t("network.localDown") };
  if (corner.remote === "down") return { signal: "bad", text: t("network.remoteDown") };
  if (corner.quality === "good") return { signal: "good", text: t("network.good") };
  if (corner.quality === "fair") return { signal: "warn", text: t("network.fair") };
  if (corner.quality === "poor") return { signal: "bad", text: t("network.poor") };
  return null;
}

type Signal = "good" | "warn" | "bad";

const tone: Record<Signal, string> = {
  good: "border-emerald-200 bg-emerald-50 text-emerald-800",
  warn: "border-amber-200 bg-amber-50 text-amber-900",
  bad: "border-red-200 bg-red-50 text-red-800",
};

const dot: Record<Signal, string> = {
  good: "bg-emerald-500",
  warn: "bg-amber-400",
  bad: "bg-red-500",
};

function StatusLine({ signal, text }: { signal: Signal; text: string }) {
  return (
    <span data-signal={signal} title={text} className={`flex min-w-0 items-center gap-2 rounded-full border px-2.5 py-1 text-sm shadow-sm ${tone[signal]}`}>
      <span aria-hidden="true" className={`h-2.5 w-2.5 shrink-0 rounded-full ${dot[signal]}`} />
      <span className="truncate">{text}</span>
    </span>
  );
}
