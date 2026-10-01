import { fireEvent, render, screen } from "@testing-library/react";

import "./i18n";
import Exhibitions, { type Exhibition } from "./Exhibitions";

const exhibitions: Exhibition[] = [
  { id: "5245081", name: "TechCrunch" },
  { id: "2", name: "Bo" },
];

function renderExhibitions(
  props: Partial<{
    exhibitions: Exhibition[];
    current: Exhibition | null;
    tokenStored: boolean;
    onSelect: (id: string, name: string) => void;
  }> = {},
) {
  return render(
    <Exhibitions
      exhibitions={props.exhibitions ?? exhibitions}
      current={props.current === undefined ? exhibitions[0] : props.current}
      tokenStored={props.tokenStored ?? false}
      onSelect={props.onSelect ?? (() => {})}
    />,
  );
}

test("exhibitions_lists_names", () => {
  renderExhibitions();
  expect(screen.getByRole("button", { name: "TechCrunch" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Bo" })).toBeInTheDocument();
  expect(
    screen.getByRole("region", { name: "Текущая выставка" }),
  ).toHaveTextContent("TechCrunch");
});

test("exhibitions_selects_one", () => {
  const onSelect = vi.fn();
  renderExhibitions({ onSelect });
  fireEvent.click(screen.getByRole("button", { name: "TechCrunch" }));
  expect(onSelect).toHaveBeenCalledTimes(1);
  expect(onSelect).toHaveBeenCalledWith("5245081", "TechCrunch");
});

test("token_stored_line", () => {
  const { rerender } = renderExhibitions({ tokenStored: true });
  const stored = screen.getByRole("status");
  expect(stored).toHaveTextContent("Токен проекта сохранён");
  expect(stored).not.toHaveTextContent("token-value");

  rerender(
    <Exhibitions
      exhibitions={exhibitions}
      current={exhibitions[0]}
      tokenStored={false}
      onSelect={() => {}}
    />,
  );
  expect(screen.getByRole("status")).toHaveTextContent(
    "Токен проекта отсутствует",
  );
});

test("keys_block_empty", () => {
  renderExhibitions();
  const keys = screen.getByRole("region", { name: "Ключи" });
  expect(keys).toHaveTextContent("Ключи");
  expect(keys).toHaveTextContent("Нет ключей");
  expect(keys.querySelector("button")).toBeNull();
});
