import path from "node:path";
import type { Connect, Plugin } from "vite";

import { foldForm } from "./app/form/fold";

function serveFoldedForm(formsDir: string): Connect.NextHandleFunction {
  return (request, response, next) => {
    const pathname = request.url?.split("?")[0];
    if (pathname !== "/forms/form.json") {
      next();
      return;
    }
    response.setHeader("Content-Type", "application/json; charset=utf-8");
    response.end(JSON.stringify(foldForm(formsDir)));
  };
}

export function foldFormPlugin(): Plugin {
  let formsDir = path.join(process.cwd(), "public", "forms");
  return {
    name: "fold-form",
    configResolved(config) {
      formsDir = path.join(config.root, "public", "forms");
    },
    configureServer(server) {
      server.middlewares.use(serveFoldedForm(formsDir));
    },
    configurePreviewServer(server) {
      server.middlewares.use(serveFoldedForm(formsDir));
    },
  };
}
