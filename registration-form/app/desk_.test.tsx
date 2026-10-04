import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { createRoutesStub } from "react-router";

import type { FormDocuments } from "./form/client";
import { foldForm } from "./form/fold";
import { readOutbox } from "./form/outbox";
import Desk, { loader } from "./routes/desk";

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
}

function mount(options: {
  documents?: FormDocuments;
  search?: unknown;
  categories?: unknown;
  printers?: unknown;
  save?: () => Response | Promise<Response>;
}) {
  const documents = options.documents ?? foldForm();
  const posts: unknown[] = [];
  const photos: string[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const raw = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      const url = new URL(raw, "http://localhost");
      const method = init?.method ?? "GET";
      if (method === "POST" && url.pathname === "/api/registrations") {
        posts.push(JSON.parse(String(init?.body)));
        return options.save ? options.save() : new Response(JSON.stringify({ id: "row-1" }), { status: 200 });
      }
      if (method === "POST" && url.pathname.endsWith("/photo")) {
        photos.push(url.pathname);
        return new Response("{}", { status: 200 });
      }
      if (url.pathname === "/forms/form.json") {
        return new Response(JSON.stringify(documents), { status: 200, headers: { "Content-Type": "application/json" } });
      }
      if (url.pathname === "/api/forms/categories") {
        return new Response(JSON.stringify(options.categories ?? []), { status: 200 });
      }
      if (url.pathname === "/api/printers") {
        return new Response(JSON.stringify(options.printers ?? []), { status: 200 });
      }
      if (url.pathname === "/api/registrations") {
        return new Response(JSON.stringify(options.search ?? []), { status: 200 });
      }
      return new Response("missing", { status: 404 });
    }),
  );
  const Stub = createRoutesStub([
    {
      path: "/desk",
      Component: Desk,
      HydrateFallback: () => null,
      loader,
    },
  ]);
  render(<Stub initialEntries={["/desk"]} />);
  return { posts, photos };
}

beforeEach(() => {
  Object.defineProperty(window, "innerWidth", { configurable: true, value: 1024 });
  Object.defineProperty(window, "innerHeight", { configurable: true, value: 768 });
  localStorage.clear();
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

test("desk_omits_search", async () => {
  mount({});
  expect(await screen.findByText("Личные данные")).toBeInTheDocument();
  expect(screen.getByLabelText("Название компании", { exact: false })).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Найти" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Новый посетитель" })).not.toBeInTheDocument();
  expect(screen.queryByLabelText("Поиск")).not.toBeInTheDocument();
});

test("desk_condition_shows_the_block", async () => {
  const documents = foldForm();
  documents.conf[1].next = [
    { step: 2, conditions: { operator: "and", expression: [{ q_id: "personalData.surname", value: "Иванов" }] } },
  ];
  mount({ documents });
  expect(await screen.findByText("Ответьте на вопросы")).toBeInTheDocument();
  expect(screen.queryByLabelText("Название компании", { exact: false })).not.toBeInTheDocument();
  fireEvent.change(screen.getByLabelText("Фамилия", { exact: false }), { target: { value: "Иванов" } });
  expect(screen.getByLabelText("Название компании", { exact: false })).toBeInTheDocument();
  fireEvent.change(screen.getByLabelText("Фамилия", { exact: false }), { target: { value: "Петров" } });
  expect(screen.queryByLabelText("Название компании", { exact: false })).not.toBeInTheDocument();
});

test("desk_condition_tree_shows_one_path", async () => {
  const documents = foldForm();
  documents.conf.length = 1;
  documents.conf[0].next = [
    { step: 1, conditions: { operator: "and", expression: [{ q_id: "personalData.surname", value: "Иванов" }] } },
    { step: 2, conditions: { operator: "and", expression: [{ q_id: "personalData.surname", value: "Петров" }] } },
  ];
  const block = (advice: string, next: unknown[]) => {
    documents.conf.push({ advice: { ru: advice }, body: [], next });
  };
  block("Блок B", [{ step: 3 }]);
  block("Блок C", [{ step: 6 }]);
  block("Блок D", [{ step: 4 }]);
  block("Блок E", [{ step: 5 }]);
  block("Блок F", [{ step: -1 }]);
  block("Блок H", [{ step: 5 }]);
  mount({ documents });
  expect(await screen.findByText("Личные данные")).toBeInTheDocument();
  for (const name of ["Блок B", "Блок C", "Блок D", "Блок E", "Блок F", "Блок H"]) {
    expect(screen.queryByText(name)).not.toBeInTheDocument();
  }
  fireEvent.change(screen.getByLabelText("Фамилия", { exact: false }), { target: { value: "Иванов" } });
  for (const name of ["Блок B", "Блок D", "Блок E", "Блок F"]) expect(screen.getByText(name)).toBeInTheDocument();
  expect(screen.queryByText("Блок C")).not.toBeInTheDocument();
  expect(screen.queryByText("Блок H")).not.toBeInTheDocument();
  fireEvent.change(screen.getByLabelText("Фамилия", { exact: false }), { target: { value: "Петров" } });
  for (const name of ["Блок C", "Блок H", "Блок F"]) expect(screen.getByText(name)).toBeInTheDocument();
  for (const name of ["Блок B", "Блок D", "Блок E"]) expect(screen.queryByText(name)).not.toBeInTheDocument();
  fireEvent.change(screen.getByLabelText("Фамилия", { exact: false }), { target: { value: "Сидоров" } });
  for (const name of ["Блок B", "Блок C", "Блок D", "Блок E", "Блок F", "Блок H"]) {
    expect(screen.queryByText(name)).not.toBeInTheDocument();
  }
});

test("desk_fields_fit_three_across", async () => {
  Object.defineProperty(window, "innerWidth", { configurable: true, value: 1024 });
  mount({});
  const surname = await screen.findByLabelText("Фамилия", { exact: false });
  expect(surname.closest("[data-testid=body-row]")).toHaveClass("min-[865px]:w-1/3");
  expect(screen.getByTestId("operator-bar")).toHaveClass("flex-nowrap");
  expect(screen.getAllByTestId("choice")[0]).toHaveClass("min-[865px]:grid-cols-3");
  expect(screen.queryByRole("button", { name: "Далее" })).not.toBeInTheDocument();
  expect(screen.getByTestId("frame")).not.toHaveClass("max-w-kiosk");
});

test("desk_save_posts_printer", async () => {
  const documents = foldForm();
  relax(documents);
  const { posts } = mount({
    documents,
    categories: [{ cat_id: 3, name: { ru: "Гость" } }],
    printers: [{ name: "Zebra", text: "Zebra" }],
  });
  await screen.findByRole("option", { name: "Гость" });
  await screen.findByRole("option", { name: "Zebra" });
  fireEvent.change(screen.getByLabelText("Категория"), { target: { value: "3" } });
  fireEvent.click(screen.getByRole("tab", { name: "Оплачен" }));
  fireEvent.change(screen.getByLabelText("Принтер"), { target: { value: "Zebra" } });
  fireEvent.change(screen.getByTestId("packets"), { target: { value: "4" } });
  fireEvent.change(screen.getByLabelText("Фамилия", { exact: false }), { target: { value: "Иванов" } });
  fireEvent.click(screen.getByRole("button", { name: "Сохранить" }));
  await waitFor(() => expect(posts).toHaveLength(1));
  const body = posts[0] as {
    printer: string;
    visitor: {
      category: number;
      ext_packets: { pack_id: number }[];
      subscribtion: { jvrel_prop: { ticket_status: number } };
      personalData: { surname: string };
    };
  };
  expect(body.printer).toBe("Zebra");
  expect(body.visitor.category).toBe(3);
  expect(body.visitor.subscribtion.jvrel_prop.ticket_status).toBe(1);
  expect(body.visitor.ext_packets[0]?.pack_id).toBe(4);
  expect(body.visitor.personalData.surname).toBe("Иванов");
});

test("desk_narrow_keeps_the_fields", async () => {
  Object.defineProperty(window, "innerWidth", { configurable: true, value: 767 });
  Object.defineProperty(window, "innerHeight", { configurable: true, value: 900 });
  mount({});
  const category = await screen.findByLabelText("Категория");
  const pay = screen.getByRole("tab", { name: "Не оплачен" });
  const printer = screen.getByLabelText("Принтер");
  const step = screen.getByTestId("step");
  expect(category.compareDocumentPosition(pay) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(pay.compareDocumentPosition(printer) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(printer.compareDocumentPosition(step) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(screen.getByTestId("operator-bar")).toHaveClass("flex-nowrap");
  expect(screen.getByTestId("frame")).toHaveClass("flex", "w-full", "flex-col", "pb-bar");
});

test("desk_does_not_idle", async () => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  mount({});
  fireEvent.change(await screen.findByLabelText("Фамилия", { exact: false }), { target: { value: "Иванов" } });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(120_000);
  });
  expect(screen.getByLabelText("Фамилия", { exact: false })).toHaveValue("Иванов");
  expect(screen.queryByRole("button", { name: "Начать" })).not.toBeInTheDocument();
});

test("desk_failed_save_stays", async () => {
  const documents = foldForm();
  relax(documents);
  const { posts } = mount({
    documents,
    save: () => new Response("no", { status: 500 }),
  });
  fireEvent.change(await screen.findByLabelText("Фамилия", { exact: false }), { target: { value: "Иванов" } });
  fireEvent.click(screen.getByRole("button", { name: "Сохранить" }));
  await waitFor(() => expect(posts).toHaveLength(1));
  expect(screen.getByLabelText("Фамилия", { exact: false })).toHaveValue("Иванов");
  expect(readOutbox()).toEqual([]);
  expect(screen.queryByRole("button", { name: "Начать" })).not.toBeInTheDocument();
});
