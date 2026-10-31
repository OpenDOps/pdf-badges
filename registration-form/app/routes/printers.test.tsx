import { fireEvent, render, screen, waitFor } from "@testing-library/react";

import "../i18n";
import { resetDeskCatalog } from "../shell/catalog";
import Printers from "./printers";

const front = {
  id: "ipp://192.0.2.10:631/ipp/print",
  name: "ECOSYS",
  hidden: false,
  default: true,
  available: true,
  zone: "",
  category: "",
  print_cert: false,
};

const gone = {
  ...front,
  id: "ipp://192.0.2.11:631/ipp/print",
  name: "Gone",
  default: false,
  available: false,
};

function json(body: unknown) {
  return new Response(JSON.stringify({ ok: true, data: body }), {
    status: 200,
    headers: { "Content-Type": "application/json" },
  });
}

function urlOf(input: RequestInfo | URL): string {
  return typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
}

beforeEach(() => {
  resetDeskCatalog();
});

afterEach(() => {
  resetDeskCatalog();
  vi.unstubAllGlobals();
});

test("printers_saves_name_visibility_and_routing", async () => {
  const patched: { url: string; body: string }[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = urlOf(input);
      if (url.includes("/api/desk/catalog")) {
        return json({ rev: 1, categories: [{ cat_id: 3, name: { ru: "Гость" } }], printers: [] });
      }
      if (init?.method === "PATCH") {
        patched.push({ url, body: String(init.body) });
        return json({ ...front, name: "Hall" });
      }
      return json([front, gone]);
    }),
  );

  render(<Printers />);
  expect(await screen.findByDisplayValue("ECOSYS")).toBeInTheDocument();
  expect(screen.getByText(front.id)).toBeInTheDocument();
  expect(screen.queryByText("Gone")).not.toBeInTheDocument();
  expect(screen.queryByText("Доступен")).not.toBeInTheDocument();
  expect(screen.queryByText("Недоступен")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Показывать" })).toHaveAttribute("aria-pressed", "true");

  fireEvent.change(screen.getByDisplayValue("ECOSYS"), { target: { value: "Hall" } });
  fireEvent.change(screen.getByLabelText("Зона"), { target: { value: "12" } });
  expect(screen.getByRole("combobox", { name: "Категория" })).toHaveValue("Нет категории");
  fireEvent.click(screen.getByRole("combobox", { name: "Категория" }));
  expect(await screen.findByRole("option", { name: "Нет категории" })).toBeInTheDocument();
  fireEvent.click(screen.getByRole("option", { name: "Гость" }));
  fireEvent.click(screen.getByRole("button", { name: "Показывать" }));
  fireEvent.click(screen.getByRole("checkbox", { name: "Печать сертификата" }));
  fireEvent.click(screen.getByRole("button", { name: "Сохранить" }));

  expect(await screen.findByText("Ok")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Показывать" })).toHaveAttribute("aria-pressed", "false");
  expect(patched).toHaveLength(1);
  expect(patched[0]?.url).toContain(encodeURIComponent(front.id));
  expect(JSON.parse(patched[0]?.body ?? "")).toMatchObject({
    name: "Hall",
    hidden: true,
    default: true,
    zone: "12",
    category: "3",
    print_cert: true,
  });
});

test("printers_polls_name_and_availability_without_dropping_an_edit", async () => {
  let rows = [front, gone];
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL) => {
      if (urlOf(input).includes("/api/desk/catalog")) {
        return json({ rev: 1, categories: [], printers: [] });
      }
      return json(rows);
    }),
  );

  render(<Printers />);
  expect(await screen.findByDisplayValue("ECOSYS")).toBeInTheDocument();
  expect(screen.queryByDisplayValue("Gone")).not.toBeInTheDocument();
  rows = [{ ...front, name: "From search", available: true }, gone];
  fireEvent.change(screen.getByDisplayValue("ECOSYS"), { target: { value: "Typing" } });
  expect(await screen.findByDisplayValue("Typing")).toBeInTheDocument();
  expect(screen.queryByDisplayValue("From search")).not.toBeInTheDocument();
  expect(screen.queryByText("Недоступен")).not.toBeInTheDocument();

  rows = [{ ...front, name: "Hall", available: true }];
  fireEvent.click(screen.getByRole("button", { name: "Сохранить" }));
  expect(await screen.findByDisplayValue("Hall")).toBeInTheDocument();
  rows = [{ ...front, name: "Later", available: true }];
  expect(await screen.findByDisplayValue("Later", {}, { timeout: 4000 })).toBeInTheDocument();
});

test("printers_clears_the_default_on_a_second_click", async () => {
  const other = { ...front, id: "ipp://192.0.2.12:631/ipp/print", name: "Other", default: false };
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL) => {
      if (urlOf(input).includes("/api/desk/catalog")) {
        return json({ rev: 1, categories: [], printers: [] });
      }
      return json([front, other]);
    }),
  );

  render(<Printers />);
  const radios = await screen.findAllByRole("radio", { name: "По умолчанию" });
  expect(radios[0]).toBeChecked();
  expect(radios[1]).not.toBeChecked();

  fireEvent.click(radios[0]!);
  expect(radios[0]).not.toBeChecked();
  expect(radios[1]).not.toBeChecked();

  fireEvent.click(radios[1]!);
  expect(radios[0]).not.toBeChecked();
  expect(radios[1]).toBeChecked();

  fireEvent.click(radios[1]!);
  expect(radios[0]).not.toBeChecked();
  expect(radios[1]).not.toBeChecked();
});

test("printers_saves_zpl_options_from_the_dialog", async () => {
  const patched: string[] = [];
  const zpl = {
    ...front,
    kind: "zpl",
    params: [
      { name: "zeMediaTracking", value: "Web", values: ["Continuous", "Web", "Mark"] },
      { name: "Speed", value: "10", values: ["1", "10"] },
    ],
  };
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = urlOf(input);
      if (url.includes("/api/desk/catalog")) {
        return json({ rev: 1, categories: [], printers: [] });
      }
      if (init?.method === "PATCH") {
        patched.push(String(init.body));
        return json({ ...zpl, params: [{ name: "zeMediaTracking", value: "Mark", values: ["Continuous", "Web", "Mark"] }, { name: "Speed", value: "10", values: ["1", "10"] }] });
      }
      return json([zpl]);
    }),
  );

  render(<Printers />);
  fireEvent.click(await screen.findByRole("button", { name: "Опции" }));
  const tracking = await screen.findByRole("combobox", { name: "Слежение за носителем" });
  expect(tracking).toHaveValue("Web");
  expect(screen.getByRole("option", { name: "Промежуток между этикетками" })).toBeInTheDocument();
  fireEvent.change(tracking, { target: { value: "Mark" } });
  fireEvent.click(screen.getByRole("button", { name: "ок" }));

  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  expect(JSON.parse(patched[0] ?? "")).toMatchObject({
    options: { zeMediaTracking: "Mark", Speed: "10" },
  });
});

test("printers_clears_a_category_binding", async () => {
  const patched: string[] = [];
  const bound = { ...front, category: "3" };
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = urlOf(input);
      if (url.includes("/api/desk/catalog")) {
        return json({ rev: 1, categories: [{ cat_id: 3, name: { ru: "Гость" } }], printers: [] });
      }
      if (init?.method === "PATCH") {
        patched.push(String(init.body));
        return json({ ...bound, category: "" });
      }
      return json([bound]);
    }),
  );

  render(<Printers />);
  const category = await screen.findByRole("combobox", { name: "Категория" });
  expect(await screen.findByDisplayValue("Гость")).toBe(category);

  fireEvent.click(category);
  fireEvent.click(await screen.findByRole("option", { name: "Нет категории" }));
  expect(category).toHaveValue("Нет категории");
  fireEvent.click(screen.getByRole("button", { name: "Сохранить" }));

  expect(await screen.findByText("Ok")).toBeInTheDocument();
  expect(JSON.parse(patched[0] ?? "")).toMatchObject({ category: "" });
});
