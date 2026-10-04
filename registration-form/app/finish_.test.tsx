import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { createRoutesStub } from "react-router";

import type { FormDocuments } from "./form/client";
import { foldForm } from "./form/fold";
import { OUTBOX_KEY, readOutbox } from "./form/outbox";
import Register, { loader } from "./routes/register";

const DESK = "Получите Ваш бейдж на стойке выдачи бейджей.";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function relax(documents: FormDocuments) {
  const clear = (source: unknown) => {
    if (!isRecord(source)) return;
    source.req = false;
    source.req_loc = {};
  };
  for (const screen of documents.conf) {
    for (const item of screen.body) {
      const group = Array.isArray(item) ? item : [item];
      group.forEach(clear);
    }
  }
  documents.hiddenq.forEach(clear);
  documents.settings = { print_on_save: true };
}

function saved(body: unknown): string {
  if (!isRecord(body) || !isRecord(body.visitor) || !isRecord(body.visitor.personalData)) return "";
  return typeof body.visitor.personalData.surname === "string" ? body.visitor.personalData.surname : "";
}

function mount(save: () => Response | Promise<Response>) {
  const documents = foldForm();
  relax(documents);
  const surnames: string[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const raw = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      const pathname = new URL(raw, "http://localhost").pathname;
      if ((init?.method ?? "GET") === "POST" && pathname === "/api/registrations") {
        surnames.push(saved(JSON.parse(String(init?.body))));
        return save();
      }
      if (pathname === "/forms/form.json") {
        return new Response(JSON.stringify(documents), {
          status: 200,
          headers: { "Content-Type": "application/json" },
        });
      }
      return new Response("missing", { status: 404 });
    }),
  );
  const Stub = createRoutesStub([
    {
      path: "/register",
      Component: Register,
      HydrateFallback: () => null,
      loader,
    },
  ]);
  render(<Stub initialEntries={["/register"]} />);
  return surnames;
}

async function start() {
  fireEvent.click(await screen.findByRole("button", { name: "Начать" }));
}

async function advancePast(label: string) {
  fireEvent.click(screen.getByRole("button", { name: "Далее" }));
  await waitFor(() => expect(screen.queryByText(label)).not.toBeInTheDocument());
}

async function finish(surname: string) {
  fireEvent.change(screen.getByLabelText("Фамилия", { exact: false }), { target: { value: surname } });
  await advancePast("Фамилия");
  await advancePast("Посещали ли Вы предыдущую выставку");
  await advancePast("Название компании");
  fireEvent.click(screen.getByRole("button", { name: "Далее" }));
}

beforeEach(() => {
  localStorage.clear();
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

test("finish_greeting_starts_the_form", async () => {
  mount(() => new Response("{}", { status: 200 }));
  expect(await screen.findByRole("button", { name: "Начать" })).toBeInTheDocument();
  expect(screen.queryByText("Личные данные")).not.toBeInTheDocument();
  await start();
  expect(screen.getByText("Личные данные")).toBeInTheDocument();
});

test("finish_desk_line_then_the_greeting", async () => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  mount(() => new Response(JSON.stringify({ zone_name: "", ticket_status: 1 }), { status: 200 }));
  await start();
  await finish("Иванов");
  expect(await screen.findByText(DESK)).toBeInTheDocument();
  await act(async () => {
    await vi.advanceTimersByTimeAsync(15_000);
  });
  expect(await screen.findByRole("button", { name: "Начать" })).toBeInTheDocument();
  expect(screen.queryByLabelText("Фамилия")).not.toBeInTheDocument();
});

test("finish_zone_replaces_the_desk_line", async () => {
  mount(() => new Response(JSON.stringify({ zone_name: "Сектор А", ticket_status: 1 }), { status: 200 }));
  await start();
  await finish("Иванов");
  expect(await screen.findByText("Сектор А")).toBeInTheDocument();
  expect(screen.queryByText(DESK)).not.toBeInTheDocument();
});

test("finish_failed_save_is_queued", async () => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  let open = false;
  const surnames = mount(() => {
    if (!open) return Promise.reject(new TypeError("Failed to fetch"));
    return new Response(JSON.stringify({ zone_name: "", ticket_status: 1 }), { status: 200 });
  });
  await start();
  await finish("Иванов");
  expect(await screen.findByRole("button", { name: "Начать" })).toBeInTheDocument();
  expect(screen.queryByText(DESK)).not.toBeInTheDocument();
  expect(readOutbox()).toHaveLength(1);
  open = true;
  await vi.advanceTimersByTimeAsync(5_000);
  await waitFor(() => expect(readOutbox()).toHaveLength(0));
  expect(surnames).toContain("Иванов");
  expect(localStorage.getItem(OUTBOX_KEY)).toBe("[]");
});

test("finish_queue_does_not_block", async () => {
  mount(() => Promise.reject(new TypeError("Failed to fetch")));
  await start();
  await finish("Первый");
  expect(await screen.findByRole("button", { name: "Начать" })).toBeInTheDocument();
  expect(readOutbox()).toHaveLength(1);
  await start();
  await finish("Второй");
  expect(await screen.findByRole("button", { name: "Начать" })).toBeInTheDocument();
  const queued = readOutbox().map((item) =>
    isRecord(item) && isRecord(item.personalData) ? item.personalData.surname : "",
  );
  expect(queued).toEqual(["Первый", "Второй"]);
});
