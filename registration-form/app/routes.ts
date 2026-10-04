import { type RouteConfig, route } from "@react-router/dev/routes";

export default [
  route("register", "routes/register.tsx"),
  route("desk", "routes/desk.tsx"),
] satisfies RouteConfig;
