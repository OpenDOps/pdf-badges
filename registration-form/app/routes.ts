import { type RouteConfig, index, layout, route } from "@react-router/dev/routes";

export default [
  route("register", "routes/register.tsx"),
  route("key", "routes/key.tsx"),
  route("desk", "routes/desk-redirect.tsx"),
  layout("routes/operator.tsx", [
    index("routes/menu.tsx"),
    route("form", "routes/desk.tsx"),
    route("visitor", "routes/visitor.tsx"),
    route("visitors", "routes/visitors.tsx"),
    route("print", "routes/print.tsx"),
    route("import", "routes/import.tsx"),
    route("settings", "routes/settings.tsx"),
    route("printers", "routes/printers.tsx"),
    route("settings/registration", "routes/registration-settings.tsx"),
  ]),
] satisfies RouteConfig;
