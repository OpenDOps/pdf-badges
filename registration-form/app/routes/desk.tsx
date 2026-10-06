import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useLoaderData, useLocation, useNavigate, useSearchParams } from "react-router";

import { FormClient, type FormDocuments } from "../form/client";
import { build, loadVisitor, type FormState } from "../form/engine";
import { Step } from "../form/step";
import { kioskFrame, StepFrameContext, type StepFrame } from "../layouts/frame";
import { chooseDeskCatalog, useDeskCatalog } from "../shell/catalog";
import { openVisitor, setVisitorDirty, useOpenVisitor, useVisitorDirty, visitorHref } from "../shell/open-visitor";
import { ClosedSelect } from "../picker";

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

function childRecord(parent: Record<string, unknown>, key: string): Record<string, unknown> {
  const current = parent[key];
  if (isRecord(current)) return current;
  const next = {};
  parent[key] = next;
  return next;
}

function textId(value: unknown): string {
  if (typeof value === "number" && Number.isFinite(value)) return String(value);
  if (typeof value === "string") return value;
  return "";
}

function ticketOf(visitor: Record<string, unknown>): string {
  const sub = visitor.subscribtion;
  if (!isRecord(sub)) return "";
  const prop = sub.jvrel_prop;
  if (!isRecord(prop)) return "";
  return textId(prop.ticket_status);
}

function packetOf(visitor: Record<string, unknown>): string {
  const packs = visitor.ext_packets;
  if (!Array.isArray(packs) || !isRecord(packs[0])) return "";
  return textId(packs[0].pack_id);
}

function overlay(base: Record<string, unknown>, incoming: Record<string, unknown>) {
  for (const [key, value] of Object.entries(incoming)) {
    const current = base[key];
    if (Array.isArray(value)) {
      const template = Array.isArray(current) ? current.find(isRecord) : undefined;
      if (value.length === 0 && Array.isArray(current) && current.length > 0) continue;
      if (template && value.every(isRecord)) {
        base[key] = value.map((item) => {
          const row = structuredClone(template);
          overlay(row, item);
          return row;
        });
        continue;
      }
    }
    if (isRecord(value) && isRecord(current)) {
      overlay(current, value);
      continue;
    }
    base[key] = value;
  }
}

function formSnapshot(visitor: Record<string, unknown>, category: string, ticket: string, packets: string) {
  return JSON.stringify({ visitor, category, ticket, packets });
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
  const navigate = useNavigate();
  const { pathname } = useLocation();
  const [searchParams] = useSearchParams();
  const userId = searchParams.get("userId")?.trim() ?? "";
  const opened = useOpenVisitor();
  const dirty = useVisitorDirty();
  const editing = pathname === "/visitor";
  const client = useMemo(() => new FormClient(), []);
  const frame = useDeskFrame();
  const [state] = useState<FormState>(() => build(form, "ru"));
  const [tick, setTick] = useState(0);
  const catalog = useDeskCatalog();
  const [editCategory, setEditCategory] = useState("");
  const [ticket, setTicket] = useState("");
  const [packets, setPackets] = useState("");
  const [saveError, setSaveError] = useState("");
  const categoryId = editing ? editCategory : catalog.category;
  const saved = useRef("");

  function noteEdits() {
    if (!editing || !saved.current) return;
    setVisitorDirty(formSnapshot(state.visitor, categoryId, ticket, packets) !== saved.current);
  }

  useEffect(() => {
    noteEdits();
  }, [editing, tick, categoryId, ticket, packets, state]);

  useEffect(() => {
    if (!editing) return;
    if (!opened) {
      if (userId) {
        openVisitor(userId);
        return;
      }
      navigate("/visitors", { replace: true });
      return;
    }
    if (userId !== opened.uid) {
      navigate(visitorHref(opened.uid), { replace: true });
      return;
    }
    let live = true;
    void client
      .registration(opened.uid)
      .then((body) => {
        if (!live || !isRecord(body)) return;
        const filled = structuredClone(form.model);
        overlay(filled, body);
        if (!textId(filled.uid)) filled.uid = opened.uid;
        const category = textId(body.category);
        const nextTicket = ticketOf(body);
        const nextPackets = packetOf(body);
        loadVisitor(state, filled);
        saved.current = formSnapshot(state.visitor, category, nextTicket, nextPackets);
        setVisitorDirty(false);
        setEditCategory(category);
        setTicket(nextTicket);
        setPackets(nextPackets);
        refresh();
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, [editing, opened, userId, client, navigate, state]);

  function refresh() {
    setTick((value) => value + 1);
  }

  async function saveVisitor(): Promise<boolean> {
    state.errors = [];
    stamp(state.visitor, categoryId, ticket, packets);
    try {
      await client.save(state.visitor, catalog.printer || undefined);
      saved.current = formSnapshot(state.visitor, categoryId, ticket, packets);
      setVisitorDirty(false);
      setSaveError("");
      return true;
    } catch (error) {
      setSaveError(businessCode(error) ?? "");
      return false;
    }
  }

  function leaveVisitor() {
    navigate("/visitors");
  }

  async function saveAndLeave() {
    if (await saveVisitor()) leaveVisitor();
  }

  async function printVisitor() {
    if (!opened) return;
    if (dirty && !(await saveVisitor())) return;
    try {
      await client.print([opened.uid], catalog.printer || undefined);
      setSaveError("");
      leaveVisitor();
    } catch (error) {
      setSaveError(businessCode(error) ?? "");
    }
  }

  return (
    <StepFrameContext.Provider value={frame}>
      <main data-screen="form" data-testid="frame" className="flex w-full flex-col pb-bar">
        <header data-testid="operator-bar" className="sticky top-[var(--desk-status,0px)] z-[60] flex w-full flex-nowrap items-end gap-3 border-b border-gray-200 bg-white px-4 py-3">
          <label className="flex min-w-0 flex-1 flex-col gap-1">
            <span className="text-xs font-medium text-gray-500">{t("desk.category")}</span>
            <ClosedSelect
              label={t("desk.category")}
              options={catalog.categories}
              value={categoryId}
              onPick={(id) => {
                if (editing) setEditCategory(id);
                else chooseDeskCatalog({ category: id });
              }}
            />
          </label>
          <div data-testid="pay" className="flex min-w-0 flex-1 flex-col gap-1">
            <span id="ticket-status" className="text-xs font-medium text-gray-500">
              {t("desk.ticket")}
            </span>
            <div className="flex w-full min-w-0" role="tablist" aria-labelledby="ticket-status">
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
            <ClosedSelect
              label={t("desk.printer")}
              options={catalog.printers}
              value={catalog.printer}
              onPick={(id) => chooseDeskCatalog({ printer: id })}
            />
          </label>
          <label className="flex min-w-0 flex-1 flex-col gap-1">
            <span className="text-xs font-medium text-gray-500">{t("desk.packets")}</span>
            <input aria-label={t("desk.packets")} data-testid="packets" value={packets} onChange={(event) => setPackets(event.target.value)} />
          </label>
        </header>
        <div className="px-4 py-4">
          <Step sheet state={state} client={client} onChange={refresh} onEdit={noteEdits} />
        </div>
        <div data-testid="save-bar" className="fixed inset-x-0 bottom-0 z-20 flex items-center justify-end gap-4 border-t border-gray-200 bg-white px-4 py-3">
          {saveError ? <p className="mr-auto text-red-700">{t(`error.${saveError}`)}</p> : null}
          <button
            type="button"
            onClick={() => {
              void (editing ? saveAndLeave() : saveVisitor());
            }}
          >
            {t("desk.save")}
          </button>
          {editing ? (
            <button type="button" onClick={() => void printVisitor()}>
              {t("desk.print")}
            </button>
          ) : null}
        </div>
        <span className="hidden" data-tick={tick} />
      </main>
    </StepFrameContext.Provider>
  );
}
