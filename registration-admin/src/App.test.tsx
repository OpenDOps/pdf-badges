import { render, screen } from "@testing-library/react";

import App from "./App";

test("app_renders", () => {
  render(<App />);
  expect(
    screen.getByRole("heading", { name: "registration-admin" }),
  ).toBeInTheDocument();
});
