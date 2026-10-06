import { createMemoryRouter, redirect, RouterProvider, type RouteObject } from "react-router";

import { loader as registerLoader } from "../routes/register";
import Register from "../routes/register";
import { loader as operatorLoader } from "../routes/operator";
import OperatorLayout from "../routes/operator";
import { loadScreen } from "./screens";

function withMobile(request: Request) {
  const next = new Headers(request.headers);
  next.delete("Sec-CH-UA-Mobile");
  return { request: new Request(request.url, { headers: next }) };
}

const fallback = <span hidden />;

function lazy(path: string): Pick<RouteObject, "lazy" | "hydrateFallbackElement"> {
  return {
    lazy: () => loadScreen(path),
    hydrateFallbackElement: fallback,
  };
}

export function deskRoutes(): RouteObject[] {
  return [
    {
      path: "register",
      Component: Register,
      loader(args) {
        return registerLoader(withMobile(args.request));
      },
    },
    { path: "key", ...lazy("/key") },
    {
      path: "desk",
      loader() {
        return redirect("/form");
      },
      Component() {
        return null;
      },
    },
    {
      loader: operatorLoader,
      Component: OperatorLayout,
      hydrateFallbackElement: fallback,
      children: [
        { index: true, ...lazy("/") },
        { path: "form", ...lazy("/form") },
        { path: "visitor", ...lazy("/visitor") },
        { path: "visitors", ...lazy("/visitors") },
        { path: "print", ...lazy("/print") },
        { path: "import", ...lazy("/import") },
        { path: "settings", ...lazy("/settings") },
        { path: "printers", ...lazy("/printers") },
        { path: "settings/registration", ...lazy("/settings/registration") },
      ],
    },
  ];
}

export function renderAt(path: string) {
  const router = createMemoryRouter(deskRoutes(), { initialEntries: [path] });
  return { router, view: <RouterProvider router={router} /> };
}
