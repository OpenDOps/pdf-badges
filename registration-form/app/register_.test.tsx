import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { createRoutesStub } from "react-router";

import type { FormDocuments } from "./form/client";
import { foldForm } from "./form/fold";
import { OUTBOX_KEY } from "./form/outbox";
import Register, { loader } from "./routes/register";

const DESK = "Получите Ваш бейдж на стойке выдачи бейджей.";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function relax(documents: FormDocuments, printOnSave: boolean) {
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
  documents.settings = { print_on_save: printOnSave };
}

function mount(url: string, documents: FormDocuments, save?: (body: unknown) => Response) {
  const posts: unknown[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const raw = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      const pathname = new URL(raw, "http://localhost").pathname;
      if ((init?.method ?? "GET") === "POST" && pathname === "/api/registrations") {
        const body: unknown = JSON.parse(String(init?.body));
        posts.push(body);
        return save ? save(body) : new Response("missing", { status: 404 });
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
  render(<Stub initialEntries={[url]} />);
  return posts;
}

async function start() {
  fireEvent.click(await screen.findByRole("button", { name: "Начать" }));
}

async function advancePast(label: string) {
  fireEvent.click(screen.getByRole("button", { name: "Далее" }));
  await waitFor(() => expect(screen.queryByText(label)).not.toBeInTheDocument());
}

async function finish(surname?: string) {
  if (surname !== undefined) {
    fireEvent.change(screen.getByLabelText("Фамилия", { exact: false }), { target: { value: surname } });
  }
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

test("register_loader_shows_the_first_step", async () => {
  mount("/register", foldForm());
  await start();
  expect(screen.getByText("Личные данные")).toBeInTheDocument();
});

test("register_print_on_save_posts", async () => {
  const documents = foldForm();
  relax(documents, true);
  const posts = mount("/register", documents, () =>
    new Response(JSON.stringify({ zone_name: "", ticket_status: 1 }), { status: 200 }),
  );
  await start();
  await finish("Иванов");
  await waitFor(() => expect(posts).toHaveLength(1));
  const body = posts[0] as { visitor: { personalData: { surname: string } } };
  expect(body.visitor.personalData.surname).toBe("Иванов");
});

test("register_end_without_the_flag", async () => {
  const documents = foldForm();
  relax(documents, false);
  const posts = mount("/register", documents);
  await start();
  await finish();
  expect(await screen.findByText(DESK)).toBeInTheDocument();
  expect(posts).toHaveLength(0);
});

test("register_idle_restores_the_model", async () => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  const documents = foldForm();
  relax(documents, false);
  mount("/register", documents);
  await start();
  fireEvent.change(screen.getByLabelText("Фамилия", { exact: false }), { target: { value: "Иванов" } });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(120_000);
  });
  expect(screen.queryByLabelText("Фамилия")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Начать" })).toBeInTheDocument();
});

test("register_reset_query", async () => {
  const documents = foldForm();
  relax(documents, false);
  mount("/register?reset=1", documents);
  await start();
  fireEvent.change(screen.getByLabelText("Фамилия", { exact: false }), { target: { value: "Иванов" } });
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 1100));
  });
  expect(screen.queryByLabelText("Фамилия")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Начать" })).toBeInTheDocument();
});

test("register_save_error_on_the_step", async () => {
  const documents = foldForm();
  relax(documents, true);
  mount(
    "/register",
    documents,
    () =>
      new Response(JSON.stringify({ ok: false, error: { code: "no_printer" } }), {
        status: 409,
        headers: { "Content-Type": "application/json" },
      }),
  );
  await start();
  await finish();
  const error = await screen.findByText("Нет принтера для печати бейджа.");
  expect(error).toHaveClass("text-red-700");
  expect(screen.getByText("Вид деятельности:")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Начать" })).not.toBeInTheDocument();
  expect(localStorage.getItem(OUTBOX_KEY)).toBeNull();
});
