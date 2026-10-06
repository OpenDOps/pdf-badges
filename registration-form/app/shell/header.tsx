import { useEffect, useRef, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Link, useBlocker, useLocation, useNavigate } from "react-router";

import { chooseDeskCatalog, useDeskCatalog } from "./catalog";
import { ClosedSelect } from "../picker";
import { NetworkCorner } from "./network";
import { closeVisitor, useOpenVisitor, useVisitorDirty, visitorHref } from "./open-visitor";
import { loadScreen } from "./screens";

const backTo: Record<string, string | undefined> = {
  "/form": "/",
  "/visitors": "/",
  "/print": "/visitors",
  "/import": "/",
  "/settings": "/",
  "/printers": "/settings",
  "/settings/registration": "/settings",
};

function ScreenLink({
  to,
  children,
  className,
  label,
  current,
}: {
  to: string;
  children: ReactNode;
  className?: string;
  label?: string;
  current?: boolean;
}) {
  const ref = useRef<HTMLAnchorElement>(null);
  useEffect(() => {
    const node = ref.current;
    if (!node) return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) void loadScreen(to);
      },
      { threshold: 0.5 },
    );
    observer.observe(node);
    return () => observer.disconnect();
  }, [to]);
  return (
    <Link ref={ref} to={to} prefetch="viewport" className={className} aria-label={label} aria-current={current ? "page" : undefined}>
      {children}
    </Link>
  );
}

const deskTabs = ["/form", "/visitor", "/visitors", "/printers"] as const;

export function isDeskNav(pathname: string) {
  return (deskTabs as readonly string[]).includes(pathname);
}

export function DeskNav() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const { pathname } = useLocation();
  const catalog = useDeskCatalog();
  const opened = useOpenVisitor();
  const dirty = useVisitorDirty();
  const visitors = pathname === "/visitors";
  const visitorOpen = pathname === "/visitor";
  const blocker = useBlocker(visitorOpen && dirty);
  const seenVisitor = useRef(visitorOpen);
  const ref = useRef<HTMLElement>(null);
  useEffect(() => {
    if (seenVisitor.current && !visitorOpen) closeVisitor();
    seenVisitor.current = visitorOpen;
  }, [visitorOpen]);
  useEffect(() => {
    const node = ref.current;
    if (!node) return;
    const apply = () => {
      document.documentElement.style.setProperty("--desk-status", `${node.offsetHeight}px`);
    };
    apply();
    if (typeof ResizeObserver === "undefined") {
      return () => document.documentElement.style.setProperty("--desk-status", "0px");
    }
    const observer = new ResizeObserver(apply);
    observer.observe(node);
    return () => {
      observer.disconnect();
      document.documentElement.style.setProperty("--desk-status", "0px");
    };
  }, []);
  const tabs = [
    { to: "/form", label: t("nav.form") },
    { to: "/visitors", label: t("nav.visitors") },
  ];
  return (
    <header ref={ref} data-testid="desk-nav" className="sticky top-0 z-[70] flex w-full min-w-0 items-center gap-2 overflow-visible bg-[#f3f4f6] px-3 py-1">
      <ScreenLink
        to="/"
        label={t("action.home")}
        className="inline-flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border border-slate-300 bg-white text-gray-900"
      >
        <HomeIcon />
      </ScreenLink>
      <nav aria-label={t("nav.label")} className="flex shrink-0 items-center gap-2">
        {tabs.map((tab) => (
          <ScreenLink
            key={tab.to}
            to={tab.to}
            current={pathname === tab.to}
            className={
              pathname === tab.to
                ? "rounded-lg border border-transparent bg-[#1d4ed8] px-3 py-1.5 text-sm font-semibold text-white"
                : "rounded-lg border border-slate-300 bg-white px-3 py-1.5 text-sm font-semibold text-gray-900"
            }
          >
            {tab.label}
          </ScreenLink>
        ))}
      </nav>
      {visitors ? (
        <label className="flex w-56 shrink-0 items-center gap-1.5">
          <PrinterIcon />
          <ClosedSelect
            label={t("desk.printer")}
            options={catalog.printers}
            value={catalog.printer}
            onPick={(id) => chooseDeskCatalog({ printer: id })}
          />
        </label>
      ) : null}
      {opened ? (
        <div
          data-testid="visitor-slot"
          className={
            visitorOpen
              ? "flex shrink-0 items-center rounded-lg border border-transparent bg-[#1d4ed8] text-white"
              : "flex shrink-0 items-center rounded-lg border border-slate-300 bg-white text-gray-900"
          }
        >
          <ScreenLink
            to={visitorHref(opened.uid)}
            current={visitorOpen}
            className="truncate px-3 py-1.5 text-sm font-semibold"
          >
            {t("nav.visitor")}
          </ScreenLink>
          <button
            type="button"
            aria-label={t("nav.close")}
            onClick={() => {
              if (visitorOpen) navigate("/visitors");
              else closeVisitor();
            }}
          >
            ×
          </button>
        </div>
      ) : null}
      <div className="ml-auto flex min-w-0 flex-1 justify-end">
        <NetworkCorner pinned={false} />
      </div>
      {blocker.state === "blocked" ? (
        <div className="fixed inset-0 z-[80] flex items-center justify-center bg-black/40 px-4" role="dialog" aria-modal="true" aria-labelledby="discard-title" data-testid="discard">
          <div className="w-full max-w-sm rounded-lg bg-white p-4 shadow">
            <p id="discard-title" className="text-gray-900">{t("nav.discard")}</p>
            <div className="mt-4 flex justify-end gap-2">
              <button type="button" data-choice="stay" onClick={() => blocker.reset()}>
                {t("nav.stay")}
              </button>
              <button
                type="button"
                onClick={() => {
                  blocker.proceed();
                }}
              >
                {t("nav.leave")}
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </header>
  );
}

function PrinterIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true" className="h-5 w-5 shrink-0 text-gray-500" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M6 9V2h12v7" />
      <path d="M6 18H4a2 2 0 0 1-2-2v-5a2 2 0 0 1 2-2h16a2 2 0 0 1 2 2v5a2 2 0 0 1-2 2h-2" />
      <path d="M6 14h12v8H6z" />
    </svg>
  );
}

function HomeIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true" className="h-5 w-5" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M15 21v-8a1 1 0 0 0-1-1h-4a1 1 0 0 0-1 1v8" />
      <path d="M3 10a2 2 0 0 1 .709-1.528l7-6a2 2 0 0 1 2.582 0l7 6A2 2 0 0 1 21 10v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" />
    </svg>
  );
}

export function DeskHeader() {
  const { t } = useTranslation();
  const { pathname } = useLocation();
  if (pathname === "/") return null;
  const back = backTo[pathname];
  return (
    <header>
      {back ? <ScreenLink to={back}>{t("action.back")}</ScreenLink> : <button type="button" disabled>{t("action.back")}</button>}
      <button type="button" disabled>
        {t("action.forward")}
      </button>
    </header>
  );
}

export function ScreenFrame({
  screen,
  title,
  className,
  children,
}: {
  screen: string;
  title?: string;
  className?: string;
  children?: ReactNode;
}) {
  return (
    <main data-screen={screen} className={className}>
      {title ? <h1>{title}</h1> : null}
      {children}
    </main>
  );
}

export { ScreenLink };
