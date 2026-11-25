import { act, fireEvent, render, screen, within } from "@testing-library/react";

import "./i18n";
import Keys from "./Keys";

type Row = { id: number; key: string; comment: string | null; is_admin: boolean };

function serve(rows: Row[]) {
  const calls: string[] = [];
  let next = 10;
  vi.stubGlobal(
    "fetch",
    vi.fn(async (url: string, init?: RequestInit) => {
      const method = init?.method ?? "GET";
      calls.push(`${method} ${url}`);
      const reply = (body: unknown) => ({ json: async () => body });
      if (url === "/api/keys" && method === "GET") {
        return reply({ ok: true, data: rows });
      }
      if (url === "/api/keys" && method === "POST") {
        const body = JSON.parse(String(init?.body)) as { is_admin: boolean };
        const row = { id: next++, key: `NEW${next}`, comment: null, is_admin: body.is_admin };
        rows.push(row);
        return reply({ ok: true, data: row });
      }
      const id = Number(url.split("/").pop());
      if (method === "PATCH") {
        const body = JSON.parse(String(init?.body)) as { key: string; comment: string };
        if (rows.some((row) => row.id !== id && row.key === body.key)) {
          return reply({ ok: false, error: { code: "key_taken" } });
        }
        return reply({ ok: true, data: {} });
      }
      if (method === "DELETE") {
        return reply({ ok: true, data: {} });
      }
      throw new Error(url);
    }),
  );
  return calls;
}

function defaults(): Row[] {
  return [
    { id: 1, key: "ADMIN01", comment: null, is_admin: true },
    { id: 2, key: "OPER001", comment: "door", is_admin: false },
  ];
}

afterEach(() => vi.unstubAllGlobals());

test("keys_lists_admin_and_operator_keys", async () => {
  serve(defaults());
  render(<Keys eventId="EVT" />);
  const admin = await screen.findByRole("region", { name: "Ключи администратора" });
  const operator = screen.getByRole("region", { name: "Ключи оператора" });
  expect(admin).toHaveTextContent("ADMIN01");
  expect(operator).toHaveTextContent("OPER001");
  expect(operator).toHaveTextContent("door");
});

test("keys_last_of_each_kind_cannot_be_deleted", async () => {
  serve(defaults());
  render(<Keys eventId="EVT" />);
  const admin = await screen.findByRole("region", { name: "Ключи администратора" });
  const operator = screen.getByRole("region", { name: "Ключи оператора" });
  expect(within(admin).getByRole("button", { name: "Удалить" })).toBeDisabled();
  expect(within(operator).getByRole("button", { name: "Удалить" })).toBeDisabled();
});

test("keys_add_admin_key_then_delete_one", async () => {
  const calls = serve(defaults());
  render(<Keys eventId="EVT" />);
  const admin = await screen.findByRole("region", { name: "Ключи администратора" });
  const operator = screen.getByRole("region", { name: "Ключи оператора" });
  await act(async () => {
    fireEvent.click(within(admin).getByRole("button", { name: /Добавить/ }));
  });
  expect(calls).toContain("POST /api/keys");
  // The new key opens in the admin editor, not under operators.
  expect(within(admin).getByLabelText("Ключ")).toHaveValue("NEW11");
  expect(within(operator).queryByLabelText("Ключ")).toBeNull();
  fireEvent.click(within(admin).getByRole("button", { name: "Отмена" }));
  expect(admin).toHaveTextContent("NEW11");
  expect(operator).not.toHaveTextContent("NEW11");
  const remove = within(admin).getAllByRole("button", { name: "Удалить" });
  expect(remove).toHaveLength(2);
  expect(remove[0]).toBeEnabled();
  await act(async () => {
    fireEvent.click(remove[0]);
  });
  expect(calls).toContain("DELETE /api/keys/1");
  expect(admin).not.toHaveTextContent("ADMIN01");
  expect(within(admin).getByRole("button", { name: "Удалить" })).toBeDisabled();
});

test("keys_add_posts_is_admin_true_for_admin_section", async () => {
  const bodies: unknown[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn(async (url: string, init?: RequestInit) => {
      if (url === "/api/keys" && (init?.method ?? "GET") === "GET") {
        return {
          json: async () => ({
            ok: true,
            data: [
              { id: 1, key: "ADMIN01", comment: null, is_admin: true },
              { id: 2, key: "OPER001", comment: null, is_admin: false },
            ],
          }),
        };
      }
      if (url === "/api/keys" && init?.method === "POST") {
        bodies.push(JSON.parse(String(init.body)));
        return {
          json: async () => ({
            ok: true,
            // Deliberately omit is_admin: the UI must still place the key under admin.
            data: { id: 9, key: "FORCED1", comment: null },
          }),
        };
      }
      throw new Error(url);
    }),
  );
  render(<Keys eventId="EVT" />);
  const admin = await screen.findByRole("region", { name: "Ключи администратора" });
  await act(async () => {
    fireEvent.click(within(admin).getByRole("button", { name: /Добавить/ }));
  });
  expect(bodies).toEqual([{ is_admin: true }]);
  expect(within(admin).getByLabelText("Ключ")).toHaveValue("FORCED1");
  expect(screen.getByRole("region", { name: "Ключи оператора" }).textContent).not.toContain(
    "FORCED1",
  );
});

test("keys_edit_saves_key_and_comment", async () => {
  const calls = serve(defaults());
  render(<Keys eventId="EVT" />);
  const operator = await screen.findByRole("region", { name: "Ключи оператора" });
  fireEvent.click(within(operator).getByRole("button", { name: "Изменить" }));
  fireEvent.change(within(operator).getByLabelText("Ключ"), { target: { value: " DESK42 " } });
  fireEvent.change(within(operator).getByLabelText("Комментарий"), { target: { value: "hall B" } });
  await act(async () => {
    fireEvent.click(within(operator).getByRole("button", { name: "Сохранить" }));
  });
  expect(calls).toContain("PATCH /api/keys/2");
  expect(operator).toHaveTextContent("DESK42");
  expect(operator).toHaveTextContent("hall B");
});

test("keys_edit_reports_a_taken_key", async () => {
  serve(defaults());
  render(<Keys eventId="EVT" />);
  const operator = await screen.findByRole("region", { name: "Ключи оператора" });
  fireEvent.click(within(operator).getByRole("button", { name: "Изменить" }));
  fireEvent.change(within(operator).getByLabelText("Ключ"), { target: { value: "ADMIN01" } });
  await act(async () => {
    fireEvent.click(within(operator).getByRole("button", { name: "Сохранить" }));
  });
  expect(screen.getByRole("alert")).toHaveTextContent("Такой ключ уже есть");
});
