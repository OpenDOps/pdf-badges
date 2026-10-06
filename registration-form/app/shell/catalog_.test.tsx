import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";

import { chooseDeskCatalog, DESK_CHOICE_KEY, resetDeskCatalog, useDeskCatalog } from "./catalog";

function View() {
  const catalog = useDeskCatalog();
  return (
    <div>
      <span data-testid="categories">{catalog.categories.map((item) => item.label).join(",")}</span>
      <span data-testid="printers">{catalog.printers.map((item) => item.label).join(",")}</span>
      <span data-testid="chosen">
        {catalog.category}/{catalog.printer}
      </span>
      <button type="button" onClick={() => chooseDeskCatalog({ category: "3", printer: "Zebra" })}>
        pick
      </button>
    </div>
  );
}

function json(body: unknown) {
  return new Response(JSON.stringify(body), { status: 200, headers: { "Content-Type": "application/json" } });
}

beforeEach(() => {
  localStorage.clear();
  resetDeskCatalog();
});

afterEach(() => {
  resetDeskCatalog();
  vi.unstubAllGlobals();
});

test("catalog_keeps_one_poll_and_the_choice", async () => {
  let generation = 1;
  let categories: unknown[] = [{ cat_id: 3, name: { ru: "Гость" } }];
  let printers: unknown[] = [{ name: "Zebra", text: "Zebra" }];
  let release: ((response: Response) => void) | undefined;
  const asked: string[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL) => {
      const raw = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      const rev = new URL(raw, "http://localhost").searchParams.get("rev") ?? "";
      asked.push(rev);
      if (rev !== String(generation)) {
        return json({ rev: generation, categories, printers });
      }
      return new Promise<Response>((resolve) => {
        release = resolve;
      });
    }),
  );

  const first = render(<View />);
  expect(await screen.findByTestId("categories")).toHaveTextContent("Гость");
  expect(screen.getByTestId("printers")).toHaveTextContent("Zebra");
  expect(screen.getByTestId("chosen").textContent).toBe("3/");
  fireEvent.click(screen.getByRole("button", { name: "pick" }));
  expect(screen.getByTestId("chosen")).toHaveTextContent("3/Zebra");
  await waitFor(() => expect(asked).toContain("1"));

  generation = 2;
  categories = [...categories, { cat_id: 7, name: { ru: "Пресса" } }];
  printers = [...printers, { name: "HP", text: "HP" }];
  release?.(json({ rev: generation, categories, printers }));
  await waitFor(() => expect(screen.getByTestId("categories")).toHaveTextContent("Гость,Пресса"));
  expect(screen.getByTestId("printers")).toHaveTextContent("Zebra,HP");
  expect(screen.getByTestId("chosen")).toHaveTextContent("3/Zebra");
  await waitFor(() => expect(asked).toContain("2"));

  first.unmount();
  render(<View />);
  expect(await screen.findByTestId("categories")).toHaveTextContent("Гость,Пресса");
  expect(screen.getByTestId("chosen")).toHaveTextContent("3/Zebra");
  expect(localStorage.getItem(DESK_CHOICE_KEY)).toBe(JSON.stringify({ category: "3", printer: "Zebra" }));
});

test("catalog_restores_the_stored_choice", async () => {
  localStorage.setItem(DESK_CHOICE_KEY, JSON.stringify({ category: "7", printer: "HP" }));
  vi.stubGlobal(
    "fetch",
    vi.fn(async () =>
      json({
        rev: 1,
        categories: [
          { cat_id: 3, name: { ru: "Гость" } },
          { cat_id: 7, name: { ru: "Пресса" } },
        ],
        printers: [
          { name: "Zebra", text: "Zebra" },
          { name: "HP", text: "HP" },
        ],
      }),
    ),
  );

  const first = render(<View />);
  await waitFor(() => expect(screen.getByTestId("chosen").textContent).toBe("7/HP"));
  first.unmount();
  resetDeskCatalog();
  const second = render(<View />);
  await waitFor(() => expect(screen.getByTestId("chosen").textContent).toBe("7/HP"));

  act(() => chooseDeskCatalog({ printer: "" }));
  expect(screen.getByTestId("chosen").textContent).toBe("7/");
  expect(localStorage.getItem(DESK_CHOICE_KEY)).toBe(JSON.stringify({ category: "7", printer: "" }));

  second.unmount();
  resetDeskCatalog();
  render(<View />);
  await waitFor(() => expect(screen.getByTestId("chosen").textContent).toBe("7/"));
});

test("catalog_drops_a_stored_choice_that_left", async () => {
  localStorage.setItem(DESK_CHOICE_KEY, JSON.stringify({ category: "9", printer: "Gone" }));
  vi.stubGlobal(
    "fetch",
    vi.fn(async () =>
      json({
        rev: 1,
        categories: [{ cat_id: 3, name: { ru: "Гость" } }],
        printers: [{ name: "Zebra", text: "Zebra" }],
      }),
    ),
  );

  render(<View />);
  await waitFor(() => expect(screen.getByTestId("chosen").textContent).toBe("3/"));
  expect(localStorage.getItem(DESK_CHOICE_KEY)).toBe(JSON.stringify({ category: "3", printer: "" }));
});
