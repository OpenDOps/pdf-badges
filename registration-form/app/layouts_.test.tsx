import { fireEvent, render, screen, within } from "@testing-library/react";
import { createRoutesStub } from "react-router";

import { foldForm } from "./form/fold";
import Register, { loader } from "./routes/register";

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

function mockViewport(width: number, height: number, pointer: "fine" | "coarse") {
  Object.defineProperty(window, "innerWidth", { configurable: true, value: width });
  Object.defineProperty(window, "innerHeight", { configurable: true, value: height });
  vi.stubGlobal(
    "matchMedia",
    (query: string) =>
      ({
        matches: query === "(pointer: coarse)" ? pointer === "coarse" : query === "(pointer: fine)" ? pointer === "fine" : false,
        media: query,
        addEventListener() {},
        removeEventListener() {},
        dispatchEvent() {
          return false;
        },
        addListener() {},
        removeListener() {},
        onchange: null,
      }) as MediaQueryList,
  );
}

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

async function openRegister(mobile: string | null) {
  const Stub = registerAt(mobile);
  render(<Stub initialEntries={["/register"]} />);
  await screen.findByRole("button", { name: "Начать" });
  fireEvent.click(screen.getByRole("button", { name: "Начать" }));
}

test("layouts_touch_768_is_ipad", async () => {
  mockViewport(768, 1024, "coarse");
  await openRegister(null);
  expect(screen.getByTestId("step")).toHaveClass("max-w-kiosk");
  for (const row of screen.getAllByTestId("body-row")) expect(row).toHaveClass("flex-row");
  expect(screen.getByTestId("bar")).toHaveClass("fixed");
});

test("layouts_pointer_768_is_web", async () => {
  mockViewport(1024, 768, "fine");
  await openRegister("?0");
  const bar = screen.getByTestId("bar");
  expect(screen.getByTestId("step")).toHaveClass("max-w-web");
  expect(bar).toHaveClass("mt-6");
  expect(bar).not.toHaveClass("fixed");
  for (const row of screen.getAllByTestId("body-row")) expect(row).toHaveClass("flex-row");
});

test("layouts_phone_header_stacks", async () => {
  mockViewport(390, 844, "coarse");
  await openRegister("?1");
  const frame = screen.getByTestId("frame");
  expect(frame).toHaveClass("flex-col");
  for (const row of within(frame).getAllByTestId("body-row")) expect(row).toHaveClass("flex-col");
  expect(within(frame).getByTestId("bar")).toHaveClass("fixed");
  expect(within(frame).getByTestId("step")).toHaveClass("pb-bar");
});

test("layouts_resize_keeps_the_step", async () => {
  mockViewport(1024, 768, "fine");
  await openRegister(null);
  const step = screen.getByTestId("step");
  expect(step).toHaveClass("max-w-web");
  fireEvent.change(screen.getByLabelText("Фамилия", { exact: false }), { target: { value: "Иванов" } });
  mockViewport(767, 768, "fine");
  fireEvent(window, new Event("resize"));
  expect(screen.getByTestId("step")).toHaveClass("max-w-kiosk");
  expect(screen.getByTestId("step")).not.toHaveClass("max-w-web");
  expect(screen.getByLabelText("Фамилия", { exact: false })).toHaveValue("Иванов");
  expect(screen.getByRole("heading", { name: "Личные данные" })).toBeInTheDocument();
  expect(screen.queryByRole("heading", { name: "Киоск" })).not.toBeInTheDocument();
});
