import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useLoaderData } from "react-router";

import { FormClient, type FormDocuments } from "../form/client";
import { build, checkStep, openScreens, type FormState } from "../form/engine";
import { Step } from "../form/step";
import { kioskFrame, StepFrameContext, type StepFrame } from "../layouts/frame";

const BUSINESS = new Set(["no_ticket_code", "no_printer", "invalid_phone_email_combo"]);

type SaveError = Error & { status?: number; code?: string };

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function deskFrame(): StepFrame {
  if (typeof window === "undefined") return "web";
  if (window.innerWidth < 768) return "phone";
  return kioskFrame();
}

function useDeskFrame(): StepFrame {
  const [frame, setFrame] = useState(deskFrame);
  useEffect(() => {
    const apply = () => setFrame(deskFrame());
    apply();
    window.addEventListener("resize", apply);
    return () => window.removeEventListener("resize", apply);
  }, []);
  return frame;
}

function businessCode(error: unknown): string | undefined {
  if (!(error instanceof Error)) return undefined;
  const status = (error as SaveError).status;
  if (status === undefined || status >= 500) return undefined;
  const code = (error as SaveError).code;
  if (!code || !BUSINESS.has(code)) return undefined;
  return code;
}

function asList(body: unknown): unknown[] {
  if (Array.isArray(body)) return body;
  if (isRecord(body) && Array.isArray(body.data)) return body.data;
  if (isRecord(body) && Array.isArray(body.list)) return body.list;
  return [];
}

function textOf(value: unknown): string {
  if (typeof value === "string") return value;
  if (typeof value === "number") return String(value);
  if (!isRecord(value)) return "";
  const ru = value.ru;
  if (typeof ru === "string") return ru;
  if (isRecord(ru) && typeof ru.str === "string") return ru.str;
  return typeof value.str === "string" ? value.str : "";
}

function optionsOf(body: unknown, kind: "category" | "printer"): { id: string; label: string }[] {
  const options: { id: string; label: string }[] = [];
  for (const item of asList(body)) {
    if (typeof item === "string") {
      options.push({ id: item, label: item });
      continue;
    }
    if (!isRecord(item)) continue;
    if (kind === "category") {
      const id = item.cat_id ?? item.id;
      const label = textOf(item.name) || textOf(id);
      if (id === undefined || id === null || label === "") continue;
      options.push({ id: String(id), label });
      continue;
    }
    const id = textOf(item.value) || textOf(item.name) || textOf(item.id);
    const label = textOf(item.text) || textOf(item.name) || id;
    if (!id) continue;
    options.push({ id, label });
  }
  return options;
}

function childRecord(parent: Record<string, unknown>, key: string): Record<string, unknown> {
  const current = parent[key];
  if (isRecord(current)) return current;
  const next = {};
  parent[key] = next;
  return next;
}

function stamp(visitor: Record<string, unknown>, category: string, ticket: string, packets: string) {
  if (category) visitor.category = Number(category);
  if (ticket) {
    const subscription = childRecord(visitor, "subscribtion");
    childRecord(subscription, "jvrel_prop").ticket_status = Number(ticket);
  }
  const packId = Number(packets);
  if (packets.trim() && Number.isFinite(packId)) {
    visitor.ext_packets = [{ _id: -1, pack_id: packId, has: 1, given: 0 }];
  }
}

export async function loader({ request }: { request: Request }) {
  return { form: await FormClient.fromRequest(request).load() };
}

export default function Desk() {
  const { form } = useLoaderData() as { form: FormDocuments };
  const { t } = useTranslation();
  const client = useMemo(() => new FormClient(), []);
  const frame = useDeskFrame();
  const [state] = useState<FormState>(() => build(form, "ru"));
  const [tick, setTick] = useState(0);
  const [categories, setCategories] = useState<{ id: string; label: string }[]>([]);
  const [printers, setPrinters] = useState<{ id: string; label: string }[]>([]);
  const [category, setCategory] = useState("");
  const [ticket, setTicket] = useState("");
  const [printer, setPrinter] = useState("");
  const [packets, setPackets] = useState("");
  const [saveError, setSaveError] = useState("");

  useEffect(() => {
    let stopped = false;
    void client
      .categories()
      .then((body) => {
        if (!stopped) setCategories(optionsOf(body, "category"));
      })
      .catch(() => undefined);
    void client
      .printers()
      .then((body) => {
        if (!stopped) setPrinters(optionsOf(body, "printer"));
      })
      .catch(() => undefined);
    return () => {
      stopped = true;
    };
  }, [client]);

  function refresh() {
    setTick((value) => value + 1);
  }

  async function saveVisitor() {
    state.step = openScreens(state);
    const errors = await checkStep(state, client);
    state.errors = errors;
    refresh();
    if (errors.length > 0) return;
    stamp(state.visitor, category, ticket, packets);
    try {
      await client.save(state.visitor, printer || undefined);
      setSaveError("");
    } catch (error) {
      setSaveError(businessCode(error) ?? "");
    }
  }

  return (
    <StepFrameContext.Provider value={frame}>
      <main data-testid="frame" className="flex w-full flex-col pb-bar">
        <header data-testid="operator-bar" className="sticky top-0 z-30 flex w-full flex-nowrap items-end gap-3 border-b border-gray-200 bg-white px-4 py-3">
          <label className="flex min-w-0 flex-1 flex-col gap-1">
            <span className="text-xs font-medium text-gray-500">{t("desk.category")}</span>
            <select aria-label={t("desk.category")} value={category} onChange={(event) => setCategory(event.target.value)}>
              <option value="" />
              {categories.map((item) => (
                <option key={item.id} value={item.id}>
                  {item.label}
                </option>
              ))}
            </select>
          </label>
          <div data-testid="pay" className="flex min-w-0 flex-1 flex-col gap-1">
            <span id="ticket-status" className="text-xs font-medium text-gray-500">
              {t("desk.ticket")}
            </span>
            <div className="flex" role="tablist" aria-labelledby="ticket-status">
              <button type="button" role="tab" aria-selected={ticket === "-1"} onClick={() => setTicket("-1")}>
                {t("desk.unpaid")}
              </button>
              <button type="button" role="tab" aria-selected={ticket === "1"} onClick={() => setTicket("1")}>
                {t("desk.paid")}
              </button>
            </div>
          </div>
          <label className="flex min-w-0 flex-1 flex-col gap-1">
            <span className="text-xs font-medium text-gray-500">{t("desk.printer")}</span>
            <select aria-label={t("desk.printer")} value={printer} onChange={(event) => setPrinter(event.target.value)}>
              <option value="" />
              {printers.map((item) => (
                <option key={item.id} value={item.id}>
                  {item.label}
                </option>
              ))}
            </select>
          </label>
          <label className="flex min-w-0 flex-1 flex-col gap-1">
            <span className="text-xs font-medium text-gray-500">{t("desk.packets")}</span>
            <input aria-label={t("desk.packets")} data-testid="packets" value={packets} onChange={(event) => setPackets(event.target.value)} />
          </label>
        </header>
        <div className="px-4 py-4">
          <Step sheet state={state} client={client} onChange={refresh} />
        </div>
        <div data-testid="save-bar" className="fixed inset-x-0 bottom-0 z-20 flex items-center justify-end gap-4 border-t border-gray-200 bg-white px-4 py-3">
          {saveError ? <p className="mr-auto text-red-700">{t(`error.${saveError}`)}</p> : null}
          <button
            type="button"
            onClick={() => {
              void saveVisitor();
            }}
          >
            {t("desk.save")}
          </button>
        </div>
        <span className="hidden" data-tick={tick} />
      </main>
    </StepFrameContext.Provider>
  );
}
