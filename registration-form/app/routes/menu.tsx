import { type MouseEvent, type PointerEvent, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { useMatches } from "react-router";

import { ScreenFrame, ScreenLink } from "../shell/header";

function fieldOf(matches: { data: unknown }[], name: string): unknown {
  for (const match of matches) {
    const data = match.data;
    if (!data || typeof data !== "object" || !(name in data)) continue;
    return (data as Record<string, unknown>)[name];
  }
  return undefined;
}

function numberOf(matches: { data: unknown }[], name: string): number | null {
  const value = fieldOf(matches, name);
  return typeof value === "number" ? value : null;
}

function flagOf(matches: { data: unknown }[], name: string): boolean {
  return fieldOf(matches, name) === true;
}

function addressesOf(matches: { data: unknown }[]): string[] {
  const value = fieldOf(matches, "addresses");
  if (!Array.isArray(value)) return [];
  return value.filter((item): item is string => typeof item === "string");
}

function qrSrc(origin: string): string {
  const base = origin || (typeof window === "undefined" ? "" : window.location.origin);
  return `/api/qr?text=${encodeURIComponent(`${base}/register`)}`;
}

const tile =
  "h-24 overflow-hidden rounded-lg border border-slate-300 bg-white shadow-sm [--spot-x:50%] [--spot-y:50%] hover:bg-[radial-gradient(circle_at_var(--spot-x)_var(--spot-y),#7dd3fc,#ffffff_62%)]";
const face =
  "flex h-full w-full items-center justify-center bg-transparent px-3 text-center text-lg font-semibold text-gray-900";

function trackSpot(event: PointerEvent<HTMLElement> | MouseEvent<HTMLElement>) {
  const bounds = event.currentTarget.getBoundingClientRect();
  event.currentTarget.style.setProperty("--spot-x", `${event.clientX - bounds.left}px`);
  event.currentTarget.style.setProperty("--spot-y", `${event.clientY - bounds.top}px`);
}

function MenuTile({ to, disabled, children }: { to: string; disabled?: boolean; children: ReactNode }) {
  return (
    <div
      className={disabled ? `${tile} cursor-not-allowed opacity-50` : tile}
      onPointerEnter={trackSpot}
      onPointerMove={trackSpot}
      onMouseMove={trackSpot}
    >
      {disabled ? (
        <button type="button" disabled className={`${face} pointer-events-none cursor-not-allowed`}>
          {children}
        </button>
      ) : (
        <ScreenLink className={face} to={to}>
          {children}
        </ScreenLink>
      )}
    </div>
  );
}

export default function Menu() {
  const { t } = useTranslation();
  const matches = useMatches();
  const waiting = numberOf(matches, "waiting");
  const admin = flagOf(matches, "admin");
  const addresses = addressesOf(matches);
  const origin = fieldOf(matches, "origin");
  return (
    <ScreenFrame
      screen="menu"
      className="box-border grid h-screen max-h-screen grid-rows-[auto_minmax(0,1fr)] overflow-hidden px-4 pb-3 pt-24 text-center"
    >
      <div className="flex items-center justify-center py-2">
        <div className="grid w-full max-w-3xl grid-cols-2 gap-3">
          <MenuTile to="/form">{t("menu.operator")}</MenuTile>
          <MenuTile to="/visitors">{t("menu.print")}</MenuTile>
          {admin ? (
            <>
              <MenuTile to="/import" disabled>{t("menu.import")}</MenuTile>
              <MenuTile to="/settings">{t("menu.settings")}</MenuTile>
            </>
          ) : null}
        </div>
      </div>
      <div className="flex min-h-0 flex-col items-center overflow-hidden">
        <div className="shrink-0">
          <p>
            {t("menu.waiting")} <span data-count="waiting">{waiting}</span>
          </p>
          {addresses.map((address) => (
            <p key={address}>
              {t("menu.address")} <span data-address={address}>{address}</span>
            </p>
          ))}
        </div>
        <div className="flex min-h-0 w-full flex-1 items-center justify-center">
          <img
            className="max-h-full max-w-full object-contain"
            src={qrSrc(typeof origin === "string" ? origin : "")}
            alt={t("menu.qr")}
          />
        </div>
      </div>
    </ScreenFrame>
  );
}
