import { render, screen, waitFor } from "@testing-library/react";

import { foldForm } from "./form/fold";
import { setUnauthorized } from "./form/client";
import { loader as operatorLoader } from "./routes/operator";
import { renderAt } from "./shell/test-router";
import { resetScreenLoads } from "./shell/screens";

const operatorPaths = ["/", "/form", "/visitors", "/print", "/import", "/settings", "/printers", "/settings/registration"];

function stubFetch(sync: number, categories = 200) {
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL) => {
      const raw = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      const pathname = new URL(raw, "http://localhost").pathname;
      if (pathname === "/api/sync") {
        const status = sync;
        const body = status === 200 ? { waiting: 0 } : { error: { code: "unauthorized" } };
        return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });
      }
      if (pathname === "/forms/form.json") {
        return new Response(JSON.stringify(foldForm()), { status: 200, headers: { "Content-Type": "application/json" } });
      }
      if (pathname === "/api/forms/categories") {
        return new Response("[]", { status: categories, headers: { "Content-Type": "application/json" } });
      }
      if (pathname === "/api/desk/catalog") {
        if (categories !== 200) {
          return new Response(JSON.stringify({ error: { code: "unauthorized" } }), {
            status: categories,
            headers: { "Content-Type": "application/json" },
          });
        }
        const rev = new URL(raw, "http://localhost").searchParams.get("rev");
        if (rev && rev !== "0") return new Promise(() => undefined);
        return new Response(JSON.stringify({ rev: 1, categories: [], printers: [] }), {
          status: 200,
          headers: { "Content-Type": "application/json" },
        });
      }
      return new Response("[]", { status: 200, headers: { "Content-Type": "application/json" } });
    }),
  );
}

beforeEach(() => {
  resetScreenLoads();
  setUnauthorized(undefined);
});

afterEach(() => {
  setUnauthorized(undefined);
  vi.unstubAllGlobals();
});

test("shell_loader_sends_the_desk_cookie", async () => {
  const seen: string[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn(async (_input: RequestInfo | URL, init?: RequestInit) => {
      seen.push(new Headers(init?.headers).get("cookie") ?? "");
      return new Response(JSON.stringify({ waiting: 0, synced: 0, addresses: [] }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      });
    }),
  );
  const request = new Request("http://localhost:5174/", { headers: { cookie: "desk=abc" } });
  await operatorLoader({ request });
  expect(seen).toEqual(["desk=abc"]);
});

test("shell_missing_cookie_opens_the_key", async () => {
  stubFetch(401);
  for (const path of operatorPaths) {
    const { router, view } = renderAt(path);
    const mounted = render(view);
    expect(await screen.findByRole("heading", { name: "Ключ" })).toBeInTheDocument();
    await waitFor(() => expect(router.state.location.pathname).toBe("/key"));
    mounted.unmount();
  }
});

test("shell_cookie_opens_every_operator_path", async () => {
  stubFetch(200);
  const screens: Record<string, string> = {
    "/": "menu",
    "/form": "form",
    "/visitors": "visitors",
    "/print": "print",
    "/import": "import",
    "/settings": "settings",
    "/printers": "printers",
    "/settings/registration": "registration",
  };
  for (const path of operatorPaths) {
    const { router, view } = renderAt(path);
    const mounted = render(view);
    expect(await screen.findByRole("main")).toHaveAttribute("data-screen", screens[path]);
    await waitFor(() => expect(router.state.location.pathname).toBe(path));
    mounted.unmount();
  }
  const { router, view } = renderAt("/desk");
  const mounted = render(view);
  expect(await screen.findByRole("main")).toHaveAttribute("data-screen", "form");
  await waitFor(() => expect(router.state.location.pathname).toBe("/form"));
  mounted.unmount();
});

test("shell_register_ignores_the_desk_cookie", async () => {
  stubFetch(401);
  const { router, view } = renderAt("/register");
  render(view);
  expect(await screen.findByRole("button", { name: "Начать" })).toBeInTheDocument();
  expect(router.state.location.pathname).toBe("/register");
  expect(screen.queryByRole("heading", { name: "Ключ" })).not.toBeInTheDocument();
});

test("shell_later_401_returns_to_the_key", async () => {
  stubFetch(200, 401);
  const { router, view } = renderAt("/form");
  render(view);
  expect(await screen.findByRole("heading", { name: "Ключ" })).toBeInTheDocument();
  await waitFor(() => expect(router.state.location.pathname).toBe("/key"));
});

test("shell_back_follows_the_table", async () => {
  stubFetch(200);
  const backs: Record<string, string> = {
    "/print": "/visitors",
    "/settings/registration": "/settings",
  };
  for (const [path, back] of Object.entries(backs)) {
    const { view } = renderAt(path);
    const mounted = render(view);
    const link = await screen.findByRole("link", { name: "Назад" });
    expect(link).toHaveAttribute("href", back);
    expect(screen.getByRole("button", { name: "Вперед" })).toBeDisabled();
    mounted.unmount();
  }
});
