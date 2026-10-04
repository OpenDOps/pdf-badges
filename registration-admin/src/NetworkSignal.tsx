import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { fetchNetwork, type NetworkStatus } from "./api";

export type Signal = "good" | "warn" | "bad";

export function useNetwork(enabled: boolean): NetworkStatus | null {
  const [network, setNetwork] = useState<NetworkStatus | null>(null);

  useEffect(() => {
    if (!enabled) {
      return;
    }
    let cancel = false;
    async function load() {
      try {
        const status = await fetchNetwork();
        if (!cancel) {
          setNetwork(status);
        }
      } catch {
        if (!cancel) {
          setNetwork({
            samples: 0,
            offline: false,
            quality: null,
            timeoutSecs: 10,
            localDown: true,
          });
        }
      }
    }
    void load();
    const timer = window.setInterval(() => void load(), 5000);
    return () => {
      cancel = true;
      window.clearInterval(timer);
    };
  }, [enabled]);

  return network;
}

export function signalOf(status: NetworkStatus | null): Signal | null {
  if (!status) {
    return null;
  }
  if (status.localDown) {
    return "bad";
  }
  if (status.samples < 2) {
    return null;
  }
  if (status.offline || status.quality === "poor" || status.quality === null) {
    return "bad";
  }
  if (status.quality === "fair") {
    return "warn";
  }
  return "good";
}

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

export default function NetworkSignal({ status }: { status: NetworkStatus | null }) {
  const { t } = useTranslation();
  const signal = signalOf(status);
  if (!signal || !status) {
    return null;
  }
  const label = status.localDown
    ? t("network.local")
    : status.offline
      ? t("network.offline")
      : t(`network.${status.quality ?? "poor"}`);

  return (
    <p
      role="status"
      aria-label={label}
      data-signal={signal}
      className={`fixed top-4 right-4 z-10 flex items-center gap-2 rounded-full border px-3 py-1.5 text-sm shadow-sm ${tone[signal]}`}
    >
      <span aria-hidden="true" className={`h-2.5 w-2.5 rounded-full ${dot[signal]}`} />
      {label}
    </p>
  );
}
