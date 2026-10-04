import { fireEvent, render, screen } from "@testing-library/react";
import { createRoutesStub } from "react-router";

import { foldForm } from "./form/fold";
import App, { ErrorBoundary } from "./root";
import Desk, { loader as deskLoader } from "./routes/desk";
import Register, { headers, loader } from "./routes/register";

beforeEach(() => {
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL) => {
      const raw = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      const pathname = new URL(raw, "http://localhost").pathname;
      if (pathname === "/forms/form.json") {
        return new Response(JSON.stringify(foldForm()), {
          status: 200,
          headers: { "Content-Type": "application/json" },
        });
      }
      return new Response("missing", { status: 404 });
    }),
  );
});

afterEach(() => {
  vi.unstubAllGlobals();
});

function withMobile(request: Request, mobile: string | null) {
  const next = new Headers(request.headers);
  if (mobile === null) next.delete("Sec-CH-UA-Mobile");
  else next.set("Sec-CH-UA-Mobile", mobile);
  return { request: new Request(request.url, { headers: next }) };
}

function registerAt(mobile: string | null) {
  return createRoutesStub([
    {
      path: "/register",
      Component: Register,
      HydrateFallback() {
        return null;
      },
      loader(args) {
        return loader(withMobile(args.request, mobile));
      },
    },
  ]);
}

test("routes_register_is_the_kiosk", async () => {
  expect(headers()).toEqual({ Vary: "Sec-CH-UA-Mobile" });

  for (const mobile of [null, "?0"] as const) {
    const Stub = registerAt(mobile);
    const view = render(<Stub initialEntries={["/register"]} />);
    expect(await screen.findByRole("button", { name: "Начать" })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Киоск" })).not.toBeInTheDocument();
    const greeting = screen.getByTestId("greeting");
    expect(greeting.style.backgroundImage).toContain("/regapp/bgs/startbg.png");
    expect(greeting.style.backgroundImage).toContain("lng=ru");
    expect(greeting.style.backgroundImage).toContain("resolution=pad");
    expect(screen.getByRole("button", { name: "ru" })).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(screen.getByRole("button", { name: "eng" }));
    expect(screen.getByRole("button", { name: "Start" })).toBeInTheDocument();
    expect(screen.queryByText("Личные данные")).not.toBeInTheDocument();
    expect(screen.getByTestId("greeting").style.backgroundImage).toContain("lng=en");
    fireEvent.click(screen.getByRole("button", { name: "ru" }));
    fireEvent.click(screen.getByRole("button", { name: "Начать" }));
    expect(screen.getByText("Личные данные")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Фамилия", { exact: false }), { target: { value: "Иванов" } });
    fireEvent.click(screen.getByRole("button", { name: "Назад" }));
    expect(screen.getByTestId("greeting")).toBeInTheDocument();
    expect(screen.queryByText("Личные данные")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Начать" }));
    expect(screen.getByLabelText("Фамилия", { exact: false })).toHaveValue("Иванов");
    view.unmount();
  }

  const Phone = registerAt("?1");
  render(<Phone initialEntries={["/register"]} />);
  expect(await screen.findByRole("heading", { name: "Телефон" })).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "eng" }));
  expect(screen.getByRole("heading", { name: "Phone" })).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "ru" }));
  fireEvent.click(screen.getByRole("button", { name: "Начать" }));
  expect(screen.getByText("Личные данные")).toBeInTheDocument();
});

test("routes_desk_is_the_operator", async () => {
  const Stub = createRoutesStub([
    {
      path: "/desk",
      Component: Desk,
      HydrateFallback() {
        return null;
      },
      loader: deskLoader,
    },
  ]);
  render(<Stub initialEntries={["/desk"]} />);
  expect(await screen.findByLabelText("Категория")).toBeInTheDocument();
  expect(screen.queryByRole("heading", { name: "Стойка оператора" })).not.toBeInTheDocument();
  expect(screen.getByText("Личные данные")).toBeInTheDocument();
});

test("routes_unknown_path_is_not_a_form", async () => {
  const Stub = createRoutesStub([
    {
      path: "/",
      Component: App,
      ErrorBoundary,
      children: [
        {
          path: "register",
          Component: Register,
          loader(args) {
            return loader(withMobile(args.request, null));
          },
        },
        {
          path: "desk",
          Component: Desk,
          loader: deskLoader,
        },
      ],
    },
  ]);
  render(<Stub initialEntries={["/other"]} />);
  expect(
    await screen.findByRole("heading", { name: "Страница не найдена" }),
  ).toBeInTheDocument();
  expect(screen.queryByRole("heading", { name: "Киоск" })).not.toBeInTheDocument();
  expect(screen.queryByRole("heading", { name: "Телефон" })).not.toBeInTheDocument();
  expect(screen.queryByRole("heading", { name: "Стойка оператора" })).not.toBeInTheDocument();
});
