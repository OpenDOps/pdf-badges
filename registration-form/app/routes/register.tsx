import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useLoaderData } from "react-router";

import { FormClient, type FormDocuments } from "../form/client";
import { build, setLocale, type FormState } from "../form/engine";
import { enqueueVisitor, readOutbox, writeOutbox } from "../form/outbox";
import { Step } from "../form/step";
import { KioskLayout } from "../layouts/kiosk";
import { PhoneLayout } from "../layouts/phone";

const IDLE_MS = 120_000;
const END_MS = 15_000;
const RETRY_MS = 5_000;

const BUSINESS = new Set(["no_ticket_code", "no_printer", "invalid_phone_email_combo"]);

type SaveError = Error & { status?: number; code?: string };

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function printOnSave(settings: unknown): boolean {
  return isRecord(settings) && settings.print_on_save === true;
}

function idleMs(request: Request): number {
  const reset = new URL(request.url).searchParams.get("reset");
  if (reset && /^\d+$/.test(reset)) return Number(reset) * 1000;
  return IDLE_MS;
}

function readSave(body: unknown): { zoneName: string; ticketStatus: number | null } {
  const record = isRecord(body) && isRecord(body.data) ? body.data : body;
  if (!isRecord(record)) return { zoneName: "", ticketStatus: null };
  const zoneName = typeof record.zone_name === "string" ? record.zone_name : "";
  const ticketStatus = typeof record.ticket_status === "number" ? record.ticket_status : null;
  return { zoneName, ticketStatus };
}

function endLine(zoneName: string, ticketStatus: number | null, translate: (key: string) => string): string {
  if (ticketStatus !== null && ticketStatus !== 1) return translate("end.pay");
  if (zoneName.trim()) return zoneName;
  return translate("end.desk");
}

function transportFailure(error: unknown): boolean {
  if (!(error instanceof Error)) return true;
  const status = (error as SaveError).status;
  if (status === undefined) return true;
  return status >= 500;
}

function businessCode(error: unknown): string | undefined {
  if (!(error instanceof Error)) return undefined;
  const code = (error as SaveError).code;
  if (!code || !BUSINESS.has(code) || transportFailure(error)) return undefined;
  return code;
}

function useIdle(ms: number, onIdle: () => void) {
  const onIdleRef = useRef(onIdle);
  onIdleRef.current = onIdle;
  useEffect(() => {
    let timer = window.setTimeout(() => onIdleRef.current(), ms);
    const reset = () => {
      window.clearTimeout(timer);
      timer = window.setTimeout(() => onIdleRef.current(), ms);
    };
    window.addEventListener("pointerdown", reset);
    window.addEventListener("mousedown", reset);
    window.addEventListener("keydown", reset);
    return () => {
      window.clearTimeout(timer);
      window.removeEventListener("pointerdown", reset);
      window.removeEventListener("mousedown", reset);
      window.removeEventListener("keydown", reset);
    };
  }, [ms]);
}

export async function loader({ request }: { request: Request }) {
  const form = await FormClient.fromRequest(request).load();
  const mobile = request.headers.get("Sec-CH-UA-Mobile");
  return {
    layout: mobile === "?1" ? ("phone" as const) : ("kiosk" as const),
    form,
    idleMs: idleMs(request),
  };
}

export function headers() {
  return { Vary: "Sec-CH-UA-Mobile" };
}

function splashUrl(locale: string, phone: boolean): string {
  const resolution = phone ? "phone" : "pad";
  return `/regapp/bgs/startbg.png?lng=${encodeURIComponent(locale)}&resolution=${resolution}&formId=`;
}

function Greeting({
  state,
  phone,
  onPick,
  onStart,
}: {
  state: FormState;
  phone: boolean;
  onPick: (locale: string) => void;
  onStart: () => void;
}) {
  const { t } = useTranslation();
  return (
    <section
      data-testid="greeting"
      className="fixed inset-0 z-20 bg-no-repeat"
      style={{ backgroundImage: `url("${splashUrl(state.locale, phone)}")`, backgroundSize: "100% auto" }}
    >
      <button type="button" className="absolute inset-0 z-0" aria-label={t("action.start", { lng: state.locale })} onClick={onStart} />
      <div data-testid="locale" className="absolute top-5 right-2.5 z-10 flex gap-8">
        <button type="button" aria-pressed={state.locale === "ru"} onClick={() => onPick("ru")}>
          ru
        </button>
        <button type="button" aria-pressed={state.locale === "en"} onClick={() => onPick("en")}>
          eng
        </button>
      </div>
    </section>
  );
}

export default function Register() {
  const { layout, form, idleMs: idle } = useLoaderData<typeof loader>();
  const documents = form as FormDocuments;
  const { t } = useTranslation();
  const [state, setState] = useState(() => build(documents, "ru"));
  const stateRef = useRef(state);
  stateRef.current = state;
  const [screenName, setScreenName] = useState<"greeting" | "form" | "end">("greeting");
  const [endMessage, setEndMessage] = useState("");
  const [saveError, setSaveError] = useState<string | null>(null);
  const client = useMemo(() => new FormClient(), []);
  const posting = useRef(false);
  useEffect(() => {
    document.documentElement.lang = state.locale;
  }, [state.locale]);

  function fresh(locale: string) {
    const next = build(documents, locale);
    setState(next);
    setSaveError(null);
    setEndMessage("");
    setScreenName("greeting");
  }

  useIdle(idle, () => fresh(stateRef.current.locale));

  useEffect(() => {
    let stopped = false;
    let running = false;
    async function tick() {
      if (stopped || running) return;
      const items = readOutbox();
      const first = items[0];
      if (first === undefined) return;
      running = true;
      try {
        await client.save(first);
        if (stopped) return;
        const rest = readOutbox();
        rest.shift();
        writeOutbox(rest);
      } catch {
        // A connection failure stays at the head of the queue.
      } finally {
        running = false;
      }
    }
    void tick();
    const id = window.setInterval(() => void tick(), RETRY_MS);
    return () => {
      stopped = true;
      window.clearInterval(id);
    };
  }, [client]);

  useEffect(() => {
    if (screenName !== "end") return;
    const id = window.setTimeout(() => fresh(stateRef.current.locale), END_MS);
    return () => window.clearTimeout(id);
  }, [screenName, documents]);

  async function onStepChange() {
    const current = stateRef.current;
    if (!current.ended || posting.current) return;
    posting.current = true;
    current.ended = false;
    setSaveError(null);
    try {
      if (!printOnSave(documents.settings)) {
        setEndMessage(endLine("", null, (key) => t(key, { lng: current.locale })));
        setScreenName("end");
        return;
      }
      const result = await client.save(current.visitor);
      const parsed = readSave(result);
      setEndMessage(endLine(parsed.zoneName, parsed.ticketStatus, (key) => t(key, { lng: current.locale })));
      setScreenName("end");
    } catch (error) {
      const code = businessCode(error);
      if (code) {
        setSaveError(code);
        return;
      }
      if (transportFailure(error)) {
        enqueueVisitor(current.visitor);
        fresh(current.locale);
        return;
      }
      setSaveError(null);
    } finally {
      posting.current = false;
    }
  }

  const body = (
    <>
      {screenName === "greeting" ? (
        <Greeting
          state={state}
          phone={layout === "phone"}
          onPick={(locale) => {
            setLocale(state, locale);
            setState(build(documents, locale));
          }}
          onStart={() => setScreenName("form")}
        />
      ) : null}
      {screenName === "end" ? <p className="text-lg">{endMessage}</p> : null}
      {screenName === "form" ? (
        <>
          {saveError ? <p className="text-red-700">{t(`error.${saveError}`, { lng: state.locale })}</p> : null}
          <Step
            state={state}
            client={client}
            onChange={() => void onStepChange()}
            onBack={() => setScreenName("greeting")}
          />
        </>
      ) : null}
    </>
  );
  if (layout === "phone") return <PhoneLayout locale={state.locale}>{body}</PhoneLayout>;
  return <KioskLayout>{body}</KioskLayout>;
}
