import { fireEvent, render, screen, waitFor } from "@testing-library/react";

import { foldForm } from "./form/fold";
import { resetDeskCatalog } from "./shell/catalog";
import { resetOpenVisitor } from "./shell/open-visitor";
import { renderAt } from "./shell/test-router";

type Row = {
  uid: string;
  email: string;
  name: string;
  surname: string;
  c_name: string;
  category: string | number;
  print_count?: number;
};

const firstPage: Row[] = [
  { uid: "a", email: "a@ex.ru", name: "Анна", surname: "Иванова", c_name: "Альфа", category: "Гость", print_count: 0 },
  { uid: "b", email: "b@ex.ru", name: "Борис", surname: "Петров", c_name: "Бета", category: 3, print_count: 2 },
  { uid: "d", email: "yanochka0405@yandex.ru", name: "Яна", surname: "Башарина", c_name: "", category: 3, print_count: 1 },
  { uid: "e", email: "@mail.ru", name: "Пусто", surname: "Поле", c_name: "", category: 3 },
];

const secondPage: Row[] = [
  { uid: "c", email: "c@ex.ru", name: "Вера", surname: "Сидорова", c_name: "Гамма", category: "Пресса" },
];

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });
}

function mount(
  rows: (query: URLSearchParams) => { total: number; rows: Row[] } = () => ({ total: firstPage.length, rows: firstPage }),
  options?: { failSave?: boolean; start?: string },
) {
  const asked: URLSearchParams[] = [];
  const posts: { path: string; body: unknown }[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const raw = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      const url = new URL(raw, "http://localhost");
      const method = init?.method ?? "GET";
      if (method === "POST" && (url.pathname === "/api/registrations" || url.pathname === "/api/print")) {
        posts.push({ path: url.pathname, body: JSON.parse(String(init?.body ?? "{}")) });
        if (options?.failSave && url.pathname === "/api/registrations") {
          return json({ ok: false, error: { code: "no_printer" } }, 409);
        }
        return json({ ok: true, data: {} });
      }
      if (url.pathname === "/api/sync") return json({ waiting: 0 });
      if (url.pathname === "/api/desk/catalog") {
        const rev = url.searchParams.get("rev");
        if (rev && rev !== "0") return new Promise(() => undefined);
        return json({
          rev: 1,
          categories: [
            { cat_id: -2, name: { ru: "Экспонент" } },
            { cat_id: 3, name: { ru: "Гость" } },
          ],
          printers: [
            { name: "HP", text: "HP" },
            { name: "Zebra", text: "Zebra" },
          ],
        });
      }
      if (url.pathname === "/forms/form.json") return json(foldForm());
      if (url.pathname.startsWith("/api/registrations/")) {
        const id = decodeURIComponent(url.pathname.slice("/api/registrations/".length));
        const visitor = structuredClone(foldForm().model) as Record<string, unknown>;
        const personal = visitor.personalData as Record<string, unknown>;
        personal.name = id === "b" ? "Борис" : "Анна";
        personal.surname = id === "b" ? "Петров" : "Иванова";
        visitor.category = 3;
        const companies = personal.companies;
        const company = Array.isArray(companies) ? companies[0] : undefined;
        const addresses = company && typeof company === "object" && company !== null && "addresses" in company ? company.addresses : undefined;
        const address = Array.isArray(addresses) ? addresses[0] : undefined;
        if (address && typeof address === "object" && address !== null) delete address.contact_phones;
        return json(visitor);
      }
      if (url.pathname === "/api/registrations") {
        asked.push(url.searchParams);
        return json(rows(url.searchParams));
      }
      return json([]);
    }),
  );
  const { router, view } = renderAt(options?.start ?? "/visitors");
  render(view);
  return { asked, router, posts };
}

beforeEach(() => {
  localStorage.clear();
  resetDeskCatalog();
  resetOpenVisitor();
});

afterEach(() => {
  resetDeskCatalog();
  resetOpenVisitor();
  vi.unstubAllGlobals();
});

test("visitors_lists_rows", async () => {
  mount();
  expect(await screen.findByRole("link", { name: "Главная" })).toHaveAttribute("href", "/");
  expect(screen.getByRole("link", { name: "Посетители" })).toHaveAttribute("aria-current", "page");
  expect(screen.getByRole("link", { name: "Форма" })).toHaveAttribute("href", "/form");
  expect(await screen.findByText("Анна")).toBeInTheDocument();
  expect(screen.queryByRole("columnheader", { name: "E-mail" })).not.toBeInTheDocument();
  for (const heading of ["Имя", "Фамилия", "Компания", "Категория"]) {
    expect(screen.getByRole("columnheader", { name: heading })).toBeInTheDocument();
  }
  const anna = screen.getByText("Анна").closest("tr");
  expect(anna).toHaveTextContent("Иванова");
  expect(anna).toHaveTextContent("Альфа");
  expect(anna).toHaveTextContent("Гость");
  expect(anna).not.toHaveTextContent("a@ex.ru");
  expect(anna).toHaveTextContent("(0)");
  expect(anna).toHaveClass("bg-white");
  const boris = screen.getByText("Борис").closest("tr");
  expect(boris).toHaveTextContent("Петров");
  expect(boris).toHaveTextContent("Бета");
  expect(boris).toHaveTextContent("Гость");
  expect(boris).toHaveTextContent("(2)");
  expect(boris).toHaveClass("bg-amber-100");
  const yana = screen.getByText("Яна").closest("tr");
  const masked = screen.getByText("ya***@yandex.ru");
  expect(yana).toContainElement(masked);
  expect(yana).toHaveTextContent("(1)");
  expect(yana).toHaveClass("bg-amber-50");
  expect(masked).toHaveClass("text-gray-400");
  expect(screen.queryByText("yanochka0405@yandex.ru")).not.toBeInTheDocument();
  const empty = screen.getByText("***@mail.ru");
  const blank = screen.getByText("Пусто").closest("tr");
  expect(blank).toContainElement(empty);
  expect(blank).toHaveTextContent("(0)");
  expect(blank).toHaveClass("bg-slate-50");
  expect(empty).toHaveClass("text-gray-400");
  expect(screen.getByRole("button", { name: "Отметить" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Печать" })).toBeInTheDocument();
});

test("visitors_search_and_category", async () => {
  const { asked } = mount();
  const search = await screen.findByLabelText("Поиск");
  expect(screen.queryByRole("button", { name: "Очистить поиск" })).not.toBeInTheDocument();
  fireEvent.change(search, { target: { value: "ива" } });
  expect(screen.getByRole("button", { name: "Очистить поиск" })).toBeInTheDocument();
  const category = screen.getByLabelText("Категория");
  fireEvent.click(category);
  fireEvent.click(await screen.findByRole("option", { name: "Гость" }));
  await waitFor(() => {
    const hit = asked.find((query) => query.get("q") === "ива" && query.get("category") === "3");
    expect(hit).toBeTruthy();
  });
  expect(screen.getByRole("option", { name: "Гость" })).toHaveAttribute("aria-selected", "true");
  expect(screen.getByRole("option", { name: "Все категории" })).toHaveAttribute("aria-selected", "false");
  expect(category).toHaveValue("Гость");
  fireEvent.click(screen.getByRole("option", { name: "Экспонент" }));
  await waitFor(() => {
    const hit = asked.find((query) => query.get("category") === "-2,3");
    expect(hit).toBeTruthy();
  });
  expect(screen.getByRole("option", { name: "Экспонент" })).toHaveAttribute("aria-selected", "true");
  expect(category).toHaveValue("Экспонент, Гость");
  fireEvent.click(screen.getByRole("option", { name: "Гость" }));
  await waitFor(() => {
    const hit = asked.find((query) => query.get("category") === "-2");
    expect(hit).toBeTruthy();
  });
  expect(screen.getByRole("option", { name: "Гость" })).toHaveAttribute("aria-selected", "false");
  fireEvent.click(screen.getByRole("option", { name: "Все категории" }));
  await waitFor(() => {
    const hit = asked.find((query) => query.get("q") === "ива" && !query.has("category"));
    expect(hit).toBeTruthy();
  });
  expect(category).toHaveValue("Все категории");
  expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
  const beforeClear = asked.length;
  fireEvent.click(screen.getByRole("button", { name: "Очистить поиск" }));
  expect(search).toHaveValue("");
  expect(screen.queryByRole("button", { name: "Очистить поиск" })).not.toBeInTheDocument();
  await waitFor(() => {
    const hit = asked.slice(beforeClear).find((query) => !query.has("q") && !query.has("category"));
    expect(hit).toBeTruthy();
  });
});

test("visitors_next_page", async () => {
  const { asked } = mount((query) => (query.get("offset") === "0" ? { total: 200, rows: firstPage } : { total: 200, rows: secondPage }));
  expect(await screen.findByText("Анна")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Далее" })).not.toBeInTheDocument();
  const bar = screen.getByTestId("action-bar");
  const prev = screen.getByRole("button", { name: "Предыдущая страница" });
  const next = screen.getByRole("button", { name: "Следующая страница" });
  expect(bar).toContainElement(prev);
  expect(bar).toContainElement(next);
  expect(prev).toBeDisabled();
  expect(screen.getByRole("button", { name: "1" })).toHaveAttribute("aria-current", "page");
  expect(screen.getByRole("button", { name: "3" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "10" })).toBeInTheDocument();
  expect(screen.getAllByText("...")).toHaveLength(1);
  expect(screen.queryByRole("button", { name: "4" })).not.toBeInTheDocument();
  fireEvent.click(next);
  expect(await screen.findByText("Вера")).toBeInTheDocument();
  expect(screen.queryByText("Анна")).not.toBeInTheDocument();
  expect(asked.some((query) => query.get("offset") === "20")).toBe(true);
  expect(screen.getByRole("button", { name: "2" })).toHaveAttribute("aria-current", "page");
  expect(screen.getByRole("button", { name: "4" })).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "5" })).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Предыдущая страница" }));
  expect(await screen.findByText("Анна")).toBeInTheDocument();
  expect(screen.queryByText("Вера")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "3" }));
  await waitFor(() => expect(asked.some((query) => query.get("offset") === "40")).toBe(true));
  fireEvent.click(screen.getByRole("button", { name: "5" }));
  await waitFor(() => expect(asked.some((query) => query.get("offset") === "80")).toBe(true));
  expect(screen.getByRole("button", { name: "5" })).toHaveAttribute("aria-current", "page");
  expect(screen.getByRole("button", { name: "1" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "3" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "7" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "10" })).toBeInTheDocument();
  expect(screen.getAllByText("...")).toHaveLength(2);
  expect(screen.queryByRole("button", { name: "2" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "8" })).not.toBeInTheDocument();
});

test("visitors_printer_and_switches", async () => {
  mount((query) => (query.get("q") === "ещё" ? { total: 0, rows: [] } : { total: firstPage.length, rows: firstPage }));
  const printer = await screen.findByLabelText("Принтер");
  fireEvent.click(printer);
  fireEvent.click(await screen.findByRole("option", { name: "HP" }));
  expect(printer).toHaveValue("HP");
  fireEvent.click(screen.getByRole("button", { name: "Отметить" }));
  fireEvent.click(screen.getByRole("button", { name: "Заказать сертификат" }));
  fireEvent.change(screen.getByLabelText("Поиск"), { target: { value: "ещё" } });
  await waitFor(() => expect(screen.queryByText("Анна")).not.toBeInTheDocument());
  expect(screen.getByLabelText("Принтер")).toHaveValue("HP");
  expect(screen.getByRole("button", { name: "Отметить" })).toHaveAttribute("aria-pressed", "true");
  expect(screen.getByRole("button", { name: "Заказать сертификат" })).toHaveAttribute("aria-pressed", "true");
});

test("visitors_page_selection", async () => {
  const { asked } = mount((query) => (query.get("offset") === "0" ? { total: 40, rows: firstPage } : { total: 40, rows: secondPage }));
  const page = await screen.findByRole("checkbox", { name: "Выбрать на странице" });
  const anna = () => screen.getByRole("checkbox", { name: "Анна Иванова" });
  const matches = screen.getByRole("button", { name: "Выбрать все" });
  expect(matches).toBeDisabled();
  fireEvent.click(anna());
  expect(anna()).toBeChecked();
  expect(page).toBePartiallyChecked();
  fireEvent.click(page);
  for (const name of ["Анна Иванова", "Борис Петров", "Яна Башарина", "Пусто Поле"]) {
    expect(screen.getByRole("checkbox", { name })).toBeChecked();
  }
  expect(page).toBeChecked();
  fireEvent.click(page);
  expect(anna()).not.toBeChecked();

  fireEvent.click(anna());
  fireEvent.click(screen.getByRole("button", { name: "Следующая страница" }));
  expect(await screen.findByRole("checkbox", { name: "Вера Сидорова" })).not.toBeChecked();
  fireEvent.click(screen.getByRole("button", { name: "Предыдущая страница" }));
  expect(await screen.findByRole("checkbox", { name: "Анна Иванова" })).not.toBeChecked();

  fireEvent.click(screen.getByRole("checkbox", { name: "Анна Иванова" }));
  fireEvent.change(screen.getByLabelText("Поиск"), { target: { value: "ан" } });
  await waitFor(() => expect(asked.some((query) => query.get("q") === "ан")).toBe(true));
  expect(screen.getByRole("checkbox", { name: "Анна Иванова" })).not.toBeChecked();
  expect(matches).toBeEnabled();
  fireEvent.click(matches);
  expect(matches).toHaveAttribute("aria-pressed", "true");
  expect(screen.getByRole("checkbox", { name: "Борис Петров" })).toBeChecked();
  fireEvent.click(screen.getByRole("button", { name: "Следующая страница" }));
  expect(await screen.findByRole("checkbox", { name: "Вера Сидорова" })).toBeChecked();
  expect(screen.getByRole("checkbox", { name: "Выбрать на странице" })).toBeChecked();
  expect(matches).toHaveAttribute("aria-pressed", "true");
  fireEvent.click(screen.getByRole("button", { name: "Предыдущая страница" }));
  expect(await screen.findByRole("checkbox", { name: "Борис Петров" })).toBeChecked();

  fireEvent.click(screen.getByLabelText("Категория"));
  fireEvent.click(await screen.findByRole("option", { name: "Гость" }));
  await waitFor(() => expect(asked.some((query) => query.get("category") === "3")).toBe(true));
  expect(screen.getByRole("checkbox", { name: "Анна Иванова" })).not.toBeChecked();
  expect(matches).toHaveAttribute("aria-pressed", "false");
  expect(matches).toBeEnabled();
});

test("visitors_opens_a_prefilled_form", async () => {
  const { router } = mount();
  expect(await screen.findByText("Анна")).toBeInTheDocument();
  expect(screen.queryByTestId("visitor-slot")).not.toBeInTheDocument();
  expect(screen.queryByRole("link", { name: "Посетитель" })).not.toBeInTheDocument();
  const visitorsTab = screen.getByRole("link", { name: "Посетители" });
  const printer = screen.getByLabelText("Принтер");
  expect(visitorsTab.compareDocumentPosition(printer) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  const anna = screen.getByText("Анна").closest("tr");
  const boris = screen.getByText("Борис").closest("tr");
  expect(anna).toBeTruthy();
  expect(boris).toBeTruthy();
  fireEvent.click(screen.getByRole("checkbox", { name: "Анна Иванова" }));
  expect(anna).toHaveAttribute("aria-selected", "false");
  fireEvent.click(anna!);
  expect(anna).toHaveAttribute("aria-selected", "true");
  expect(anna).toHaveClass("bg-sky-200");
  expect(router.state.location.pathname).toBe("/visitors");
  fireEvent.click(boris!);
  expect(boris).toHaveAttribute("aria-selected", "true");
  expect(anna).toHaveAttribute("aria-selected", "false");
  fireEvent.click(boris!);
  await waitFor(() => expect(router.state.location.pathname).toBe("/visitor"));
  expect(router.state.location.search).toBe("?userId=b");
  expect(screen.getByRole("link", { name: "Посетитель" })).toHaveAttribute("href", "/visitor?userId=b");
  expect(screen.getByRole("link", { name: "Посетитель" })).toHaveAttribute("aria-current", "page");
  expect(screen.getByTestId("desk-nav").querySelector("label")).toBeNull();
  expect(screen.getAllByLabelText("Принтер")).toHaveLength(1);
  expect(await screen.findByLabelText("Имя", { exact: false })).toHaveValue("Борис");
  expect(screen.getByLabelText("Фамилия", { exact: false })).toHaveValue("Петров");
  expect(screen.getByLabelText("Категория")).toHaveValue("Гость");
  fireEvent.click(screen.getByRole("button", { name: "Закрыть" }));
  await waitFor(() => expect(router.state.location.pathname).toBe("/visitors"));
  expect(screen.queryByRole("link", { name: "Посетитель" })).not.toBeInTheDocument();
  expect(screen.queryByTestId("visitor-slot")).not.toBeInTheDocument();
  expect(screen.getAllByLabelText("Принтер")).toHaveLength(1);
});

async function openBoris() {
  const { router } = mount();
  const boris = (await screen.findByText("Борис")).closest("tr");
  fireEvent.click(boris!);
  fireEvent.click(boris!);
  await waitFor(() => expect(router.state.location.pathname).toBe("/visitor"));
  expect(await screen.findByLabelText("Фамилия", { exact: false })).toHaveValue("Петров");
  return router;
}

test("visitors_keeps_the_visitor_after_reload", async () => {
  mount(() => ({ total: firstPage.length, rows: firstPage }), { start: "/visitor?userId=b" });
  expect(await screen.findByLabelText("Имя", { exact: false })).toHaveValue("Борис");
  expect(screen.getByLabelText("Фамилия", { exact: false })).toHaveValue("Петров");
  expect(screen.getByRole("link", { name: "Посетитель" })).toHaveAttribute("aria-current", "page");
  expect(screen.getByRole("link", { name: "Посетитель" })).toHaveAttribute("href", "/visitor?userId=b");
});

test("visitors_closes_the_tab_on_leave", async () => {
  const router = await openBoris();
  fireEvent.click(screen.getByRole("link", { name: "Форма" }));
  await waitFor(() => expect(router.state.location.pathname).toBe("/form"));
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(screen.queryByRole("link", { name: "Посетитель" })).not.toBeInTheDocument();

  fireEvent.click(screen.getByRole("link", { name: "Посетители" }));
  const boris = (await screen.findByText("Борис")).closest("tr");
  fireEvent.click(boris!);
  fireEvent.click(boris!);
  await waitFor(() => expect(router.state.location.pathname).toBe("/visitor"));
  const surname = await screen.findByLabelText("Фамилия", { exact: false });
  expect(surname).toHaveValue("Петров");
  fireEvent.change(surname, { target: { value: "Новый" } });
  fireEvent.click(screen.getByRole("link", { name: "Посетители" }));
  expect(await screen.findByRole("dialog")).toHaveTextContent("Изменения будут потеряны");
  expect(router.state.location.pathname).toBe("/visitor");
  fireEvent.click(screen.getByRole("button", { name: "Остаться" }));
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(router.state.location.pathname).toBe("/visitor");
  expect(screen.getByLabelText("Фамилия", { exact: false })).toHaveValue("Новый");
  fireEvent.click(screen.getByRole("button", { name: "Закрыть" }));
  expect(await screen.findByRole("dialog")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Продолжить" }));
  await waitFor(() => expect(router.state.location.pathname).toBe("/visitors"));
  expect(screen.queryByRole("link", { name: "Посетитель" })).not.toBeInTheDocument();
});

async function openBorisOn(router: { state: { location: { pathname: string } } }) {
  const boris = (await screen.findByText("Борис")).closest("tr");
  fireEvent.click(boris!);
  fireEvent.click(boris!);
  await waitFor(() => expect(router.state.location.pathname).toBe("/visitor"));
  expect(await screen.findByLabelText("Фамилия", { exact: false })).toHaveValue("Петров");
}

test("visitors_saves_and_prints", async () => {
  const { router, posts } = mount();
  await openBorisOn(router);
  expect(screen.getByRole("button", { name: "Сохранить" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Печать" })).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Печать" }));
  await waitFor(() => expect(router.state.location.pathname).toBe("/visitors"));
  expect(posts.map((post) => post.path)).toEqual(["/api/print"]);
  expect(posts[0]?.body).toMatchObject({ ids: ["b"] });
  expect(screen.queryByRole("link", { name: "Посетитель" })).not.toBeInTheDocument();

  const again = (await screen.findByText("Борис")).closest("tr");
  fireEvent.click(again!);
  fireEvent.click(again!);
  await waitFor(() => expect(router.state.location.pathname).toBe("/visitor"));
  const surname = await screen.findByLabelText("Фамилия", { exact: false });
  fireEvent.change(surname, { target: { value: "Новый" } });
  posts.length = 0;
  fireEvent.click(screen.getByRole("button", { name: "Сохранить" }));
  await waitFor(() => expect(router.state.location.pathname).toBe("/visitors"));
  expect(posts.map((post) => post.path)).toEqual(["/api/registrations"]);
  expect(JSON.stringify(posts[0]?.body)).toContain("Новый");
  expect(screen.queryByRole("link", { name: "Посетитель" })).not.toBeInTheDocument();

  const third = (await screen.findByText("Борис")).closest("tr");
  fireEvent.click(third!);
  fireEvent.click(third!);
  await waitFor(() => expect(router.state.location.pathname).toBe("/visitor"));
  fireEvent.change(await screen.findByLabelText("Фамилия", { exact: false }), { target: { value: "Новый" } });
  posts.length = 0;
  fireEvent.click(screen.getByRole("button", { name: "Печать" }));
  await waitFor(() => expect(router.state.location.pathname).toBe("/visitors"));
  expect(posts.map((post) => post.path)).toEqual(["/api/registrations", "/api/print"]);
});

test("visitors_print_stays_when_save_fails", async () => {
  const { router, posts } = mount(() => ({ total: firstPage.length, rows: firstPage }), { failSave: true });
  await openBorisOn(router);
  fireEvent.change(screen.getByLabelText("Фамилия", { exact: false }), { target: { value: "Новый" } });
  fireEvent.click(screen.getByRole("button", { name: "Печать" }));
  expect(await screen.findByText("Нет принтера для печати бейджа.")).toBeInTheDocument();
  expect(router.state.location.pathname).toBe("/visitor");
  expect(posts.map((post) => post.path)).toEqual(["/api/registrations"]);
  expect(screen.getByRole("link", { name: "Посетитель" })).toBeInTheDocument();
});

test("visitors_one_search_field", async () => {
  mount();
  expect(await screen.findByLabelText("Поиск")).toBeInTheDocument();
  expect(screen.getAllByRole("textbox")).toHaveLength(1);
  expect(screen.queryByLabelText("E-mail")).not.toBeInTheDocument();
  expect(screen.queryByLabelText("Фамилия")).not.toBeInTheDocument();
  expect(screen.queryByLabelText("Компания")).not.toBeInTheDocument();
});
