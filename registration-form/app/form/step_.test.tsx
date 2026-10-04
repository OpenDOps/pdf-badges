import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";

import { StepFrameContext } from "../layouts/frame";
import type { FormDocuments } from "./client";
import { build, setLocale, setValue } from "./engine";
import { foldForm } from "./fold";
import { Step } from "./step";

const client = { counts: async () => ({ count: 0 }) };

function separate(documents: FormDocuments) {
  documents.conf.forEach((screen, index) => {
    screen.screen_block = index + 1;
  });
}

function renderStep(documents = foldForm()) {
  render(<Step state={build(documents, "ru")} client={client} />);
}

function markedLabel(control: HTMLElement): HTMLElement {
  const label = control.closest("[data-question]")?.querySelector("label");
  if (!(label instanceof HTMLElement)) throw new Error("label");
  return label;
}

test("step_frame_walks_one_screen", async () => {
  const state = build(foldForm(), "ru");
  setValue(state, "personalData.companies.addresses.city_db", "ru", 17849);
  render(
    <StepFrameContext.Provider value="ipad">
      <Step state={state} client={client} />
    </StepFrameContext.Provider>,
  );
  expect(screen.getByRole("heading", { name: "Личные данные" })).toBeInTheDocument();
  expect(screen.queryByRole("heading", { name: "Ответьте на вопросы" })).not.toBeInTheDocument();
  fireEvent.change(screen.getByLabelText("Фамилия", { exact: false }), { target: { value: "Иванов" } });
  fireEvent.change(screen.getByLabelText("Имя", { exact: false }), { target: { value: "Иван" } });
  fireEvent.change(screen.getByLabelText("E-mail", { exact: false }), { target: { value: "ivan@example.com" } });
  fireEvent.click(screen.getByRole("button", { name: "Далее" }));
  expect(await screen.findByRole("heading", { name: "Ответьте на вопросы" })).toBeInTheDocument();
  expect(screen.queryByRole("heading", { name: "Личные данные" })).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Назад" }));
  expect(screen.getByRole("heading", { name: "Личные данные" })).toBeInTheDocument();
  expect(screen.getByLabelText("Фамилия", { exact: false })).toHaveValue("Иванов");
});

test("step_single_choices_are_radios", () => {
  const state = build(foldForm(), "ru");
  const survey = state.screens.find((screen) =>
    screen.body.some((item) => (Array.isArray(item) ? item : [item]).some((source) => source.id === "opt_8")),
  );
  const ids = (survey?.body ?? []).flatMap((item) => (Array.isArray(item) ? item : [item]).map((source) => source.id));
  expect(ids).toEqual(["opt_6", "opt_1", "opt_2", "opt_8", "opt_5", "ctxt_1"]);
  const { container } = render(<Step state={state} client={client} />);
  const use = container.querySelector('[data-question="opt_8"]');
  const level = container.querySelector('[data-question="opt_5"]');
  const heard = container.querySelector('[data-question="opt_1"]');
  expect(use?.querySelectorAll('input[type="radio"]')).toHaveLength(2);
  expect(level?.querySelectorAll('input[type="radio"]')).toHaveLength(7);
  expect(use?.querySelector("select")).toBeNull();
  expect(heard?.querySelector("input[type=checkbox]")).toBeTruthy();
  const first = level?.querySelector('input[type="radio"]');
  if (!(first instanceof HTMLInputElement)) throw new Error("radio missing");
  fireEvent.click(first);
  expect(state.visitor.opt_5).toMatchObject({ o_val: 0 });
});

test("step_phone_mask_and_country_search", async () => {
  const state = build(foldForm(), "ru");
  const catalog = {
    counts: async () => ({ count: 0 }),
    enums: async (_locale: string, name: string) => {
      if (name === "country") {
        return {
          list: [
            { id: 219, v: "Россия", d: true },
            { id: 221, v: "Беларусь" },
            { id: 222, v: "Казахстан" },
          ],
          error: null,
        };
      }
      if (name === "region") return { list: [{ id: 3948, v: "Москва", d: true }], error: null };
      if (name === "phone_code") {
        return {
          list: [
            { id: 219, v: "+7", d: true },
            { id: 221, v: "+375" },
            { id: 222, v: "+7" },
          ],
          error: null,
        };
      }
      if (name === "phone_mask") {
        return {
          list: [
            { id: 219, v: "+7 (ddd) ddd-dd-dd", d: true },
            { id: 221, v: "+375 (dd) ddd-dd-dd" },
            { id: 222, v: "+7 (ddd) ddd dddd" },
          ],
          error: null,
        };
      }
      return { list: [], error: null };
    },
  };
  render(<Step state={state} client={catalog} />);
  expect(await screen.findByRole("button", { name: "+7" })).toBeInTheDocument();
  const phone = screen.getByTestId("phone") as HTMLInputElement;
  fireEvent.focus(phone);
  expect(phone).toHaveValue("(___) ___-__-__");
  fireEvent.change(phone, { target: { value: "(495" } });
  expect(phone).toHaveValue("(495) ___-__-__");
  phone.setSelectionRange(phone.value.length, phone.value.length);
  fireEvent.keyDown(phone, { key: "Backspace" });
  expect(phone).toHaveValue("(49_) ___-__-__");
  expect(phone.selectionStart).toBe("(49_) ___-__-__".indexOf("_"));
  fireEvent.change(phone, { target: { value: "(495" } });
  fireEvent.mouseDown(screen.getByRole("button", { name: "+7" }));
  const search = screen.getByTestId("phone-search");
  expect(screen.queryByTestId("phone")).not.toBeInTheDocument();
  fireEvent.change(search, { target: { value: "Бел" } });
  const option = await screen.findByRole("option", { name: "Беларусь +375" });
  expect(screen.queryByRole("option", { name: "Россия +7" })).not.toBeInTheDocument();
  fireEvent.mouseDown(option);
  expect(await screen.findByRole("button", { name: "+375" })).toBeInTheDocument();
  expect(screen.getByTestId("phone")).toHaveValue("(49) 5");
  const personal = state.visitor.personalData as {
    companies: { addresses: { contact_phones: { str_number: string }[] }[] }[];
  };
  expect(personal.companies[0].addresses[0].contact_phones[0].str_number).toBe("+375495");
});

test("step_shows_the_advice", () => {
  renderStep();
  const step = screen.getByTestId("step");
  expect(step).toHaveClass("flex", "flex-col");
  expect(within(step).getByRole("heading", { name: "Личные данные" })).toBeInTheDocument();
});

test("step_row_is_one_group", () => {
  const documents = foldForm();
  const body = documents.conf[0].body;
  body.splice(0, 2, [body[0], body[1]]);
  renderStep(documents);
  const group = screen.getByRole("group");
  expect(group).toHaveClass("flex", "flex-col", "gap-4");
  expect(markedLabel(within(group).getByLabelText("Фамилия", { exact: false })).textContent).toBe("Фамилия *");
  expect(markedLabel(within(group).getByLabelText("Имя", { exact: false })).textContent).toBe("Имя *");
  expect(within(group).queryByText("Город")).not.toBeInTheDocument();
});

test("step_hidden_is_omitted", () => {
  renderStep();
  expect(screen.queryByText("Страна")).not.toBeInTheDocument();
  expect(screen.queryByText("Область")).not.toBeInTheDocument();
  expect(screen.queryByText("Страна проживания")).not.toBeInTheDocument();
});

test("step_place_phone_email_choice", () => {
  renderStep();
  const place = screen.getByTestId("place");
  const phone = screen.getByTestId("phone");
  const email = screen.getByTestId("email");
  expect(place.tagName).toBe("INPUT");
  expect(place).toHaveClass("w-full");
  expect(markedLabel(screen.getByLabelText("Город", { exact: false }))).toHaveClass("block");
  expect(markedLabel(screen.getByLabelText("Город", { exact: false })).textContent).toBe("Город *");
  expect(phone.tagName).toBe("INPUT");
  expect(phone).toHaveClass("w-full");
  expect(screen.getByText("Номер телефона")).toHaveClass("block");
  expect(screen.getByText("Номер телефона").textContent).toBe("Номер телефона");
  expect(email.tagName).toBe("INPUT");
  expect(email).toHaveClass("w-full");
  expect(markedLabel(screen.getByLabelText("E-mail", { exact: false }))).toHaveClass("block");
  expect(markedLabel(screen.getByLabelText("E-mail", { exact: false })).textContent).toBe("E-mail *");
  const choices = screen.getAllByTestId("choice");
  expect(choices.length).toBeGreaterThan(0);
  expect(choices.every((node) => node.classList.contains("w-full"))).toBe(true);
});

test("step_city_filters_the_catalog", async () => {
  const state = build(foldForm(), "ru");
  const catalog = {
    counts: async () => ({ count: 0 }),
    enums: async (_locale: string, name: string) => {
      if (name === "country") return { list: [{ id: 219, v: "Россия" }], error: null };
      if (name === "region") return { list: [{ id: 3948, v: "Москва", d: true }], error: null };
      return {
        list: [
          { id: 17849, v: "Москва" },
          { id: 1, v: "Московский" },
        ],
        error: null,
      };
    },
  };
  render(<Step state={state} client={catalog} />);
  const city = screen.getByLabelText("Город", { exact: false });
  fireEvent.change(city, { target: { value: "Московс" } });
  const match = await screen.findByRole("option", { name: "Московский" });
  expect(screen.queryByRole("option", { name: "Москва" })).not.toBeInTheDocument();
  fireEvent.mouseDown(match);
  await waitFor(() => {
    const personal = state.visitor.personalData as {
      companies: { addresses: { city_db: { e_val: number }; region_db: { e_val: number } }[] }[];
    };
    expect(personal.companies[0].addresses[0].city_db.e_val).toBe(1);
    expect(personal.companies[0].addresses[0].region_db.e_val).toBe(3948);
  });
  expect(city).toHaveValue("Московский");
});

test("step_city_matches_a_word_and_keeps_the_default", async () => {
  const state = build(foldForm(), "ru");
  const catalog = {
    counts: async () => ({ count: 0 }),
    enums: async (_locale: string, name: string) => {
      if (name === "country") return { list: [{ id: 219, v: "Россия" }], error: null };
      if (name === "region") return { list: [{ id: 3948, v: "Москва", d: true }], error: null };
      return {
        list: [
          { id: 2, v: "Змеиногорск" },
          { id: 3, v: "Бородино (Московская обл.)" },
          { id: 17849, v: "Москва" },
        ],
        error: null,
      };
    },
  };
  render(<Step state={state} client={catalog} />);
  const city = screen.getByLabelText("Город", { exact: false });
  fireEvent.focus(city);
  const names = async () => {
    const list = await screen.findByRole("listbox");
    return within(list).getAllByRole("option").map((option) => option.textContent);
  };
  expect(await names()).toEqual(["Моя страна не Россия", "Москва"]);
  const opened = await screen.findByRole("listbox");
  expect(within(opened).getByRole("option", { name: "Моя страна не Россия" })).toHaveClass("bg-amber-100");
  expect(within(opened).getByRole("option", { name: "Москва" })).toHaveClass("bg-sky-100", "font-semibold");
  fireEvent.change(city, { target: { value: "М" } });
  expect(await names()).toEqual(["Моя страна не Россия", "Москва", "Бородино (Московская обл.)"]);
  fireEvent.change(city, { target: { value: "Б" } });
  expect(await names()).toEqual(["Моя страна не Россия", "Бородино (Московская обл.)"]);
});

test("step_city_orders_by_weight", async () => {
  const state = build(foldForm(), "ru");
  const catalog = {
    counts: async () => ({ count: 0 }),
    enums: async (_locale: string, name: string) => {
      if (name === "country") return { list: [{ id: 219, v: "Россия" }], error: null };
      if (name === "region") return { list: [{ id: 3948, v: "Москва", d: true }], error: null };
      return {
        list: [
          { id: 8, v: "Мытищи", weight: 5 },
          { id: 9, v: "Магнитогорск", weight: 30 },
          { id: 17849, v: "Москва", d: true, weight: 100 },
        ],
        error: null,
      };
    },
  };
  render(<Step state={state} client={catalog} />);
  const city = screen.getByLabelText("Город", { exact: false });
  fireEvent.change(city, { target: { value: "М" } });
  const list = await screen.findByRole("listbox");
  expect(within(list).getAllByRole("option").map((option) => option.textContent)).toEqual([
    "Моя страна не Россия",
    "Москва",
    "Магнитогорск",
    "Мытищи",
  ]);
});

test("step_city_other_country_follows_the_language", async () => {
  const state = build(foldForm(), "ru");
  const catalog = {
    counts: async () => ({ count: 0 }),
    enums: async (locale: string, name: string) => {
      if (name === "country") {
        return {
          list: [
            { id: 219, v: locale === "en" ? "Russia" : "Россия" },
            { id: 221, v: locale === "en" ? "Belarus" : "Беларусь" },
          ],
          error: null,
        };
      }
      if (name === "region") return { list: [{ id: 3948, v: "Москва", d: true }], error: null };
      return { list: [{ id: 17849, v: "Москва", d: true }], error: null };
    },
  };
  const view = render(<Step state={state} client={catalog} />);
  const city = screen.getByLabelText("Город", { exact: false });
  fireEvent.focus(city);
  const choice = await screen.findByRole("option", { name: "Моя страна не Россия" });
  expect(choice).toHaveClass("bg-amber-100", "font-semibold");
  expect(screen.getByRole("option", { name: "Москва" })).toHaveClass("bg-sky-100", "font-semibold");
  setLocale(state, "en");
  view.rerender(<Step state={state} client={catalog} />);
  expect(await screen.findByRole("option", { name: "My country is not Russia" })).toBeInTheDocument();
  setLocale(state, "ru");
  view.rerender(<Step state={state} client={catalog} />);
  fireEvent.mouseDown(await screen.findByRole("option", { name: "Моя страна не Россия" }));
  const country = await screen.findByLabelText("Страна", { exact: false });
  expect(country.tagName).toBe("INPUT");
  expect(country).toHaveAttribute("role", "combobox");
  const stack = country.parentElement?.parentElement?.parentElement;
  expect(stack).toHaveClass("flex-col");
  expect(stack).toContainElement(screen.getByLabelText("Город", { exact: false }));
  expect(stack).not.toContainElement(screen.getByLabelText("Фамилия", { exact: false }));
  await waitFor(() => expect(country).toHaveValue("Россия"));
  expect(country).toHaveFocus();
  expect(screen.getByRole("option", { name: "Россия" })).toHaveClass("bg-sky-100", "font-semibold");
  fireEvent.change(country, { target: { value: "Бел" } });
  fireEvent.mouseDown(await screen.findByRole("option", { name: "Беларусь" }));
  await waitFor(() => {
    const personal = state.visitor.personalData as {
      companies: { addresses: { country_db: { e_val: number } }[] }[];
    };
    expect(personal.companies[0].addresses[0].country_db.e_val).toBe(221);
  });
  fireEvent.focus(screen.getByLabelText("Город", { exact: false }));
  expect(await screen.findByRole("option", { name: "Моя страна не Беларусь" })).toBeInTheDocument();
});

test("step_city_uses_the_country_already_on_the_step", async () => {
  const documents = foldForm();
  const source = documents.hiddenq.find((item) => {
    return !!item && typeof item === "object" && "id" in item && item.id === "personalData.companies.addresses.country_db";
  });
  if (!source) throw new Error("missing country");
  documents.conf[0].body.unshift(source);
  const state = build(documents, "ru");
  const catalog = {
    counts: async () => ({ count: 0 }),
    enums: async (_locale: string, name: string) => {
      if (name === "country") return { list: [{ id: 219, v: "Россия" }, { id: 221, v: "Беларусь" }], error: null };
      if (name === "region") return { list: [{ id: 3948, v: "Москва", d: true }], error: null };
      return { list: [{ id: 17849, v: "Москва", d: true }], error: null };
    },
  };
  render(<Step state={state} client={catalog} />);
  expect(screen.getAllByLabelText("Страна", { exact: false })).toHaveLength(1);
  const city = screen.getByLabelText("Город", { exact: false });
  fireEvent.focus(city);
  fireEvent.mouseDown(await screen.findByRole("option", { name: "Моя страна не Россия" }));
  expect(screen.getAllByLabelText("Страна", { exact: false })).toHaveLength(1);
  const country = screen.getByLabelText("Страна", { exact: false });
  expect(country).toHaveFocus();
  await waitFor(() => expect(country).toHaveValue("Россия"));
});

test("step_chrome_follows_the_language", async () => {
  const state = build(foldForm(), "en");
  render(<Step state={state} client={client} />);
  fireEvent.click(screen.getByRole("button", { name: "Next" }));
  expect(await screen.findByText("Fill in this field")).toBeInTheDocument();
  expect(screen.queryByText("Заполните это поле")).not.toBeInTheDocument();
});

test("step_error_sits_on_the_question", async () => {
  renderStep();
  fireEvent.click(screen.getByRole("button", { name: "Далее" }));
  const error = await screen.findByText("Заполните это поле");
  expect(error).toHaveClass("text-red-700");
  expect(error.closest("[data-question]")).toHaveAttribute("data-question", "personalData.surname");
  expect(screen.getByRole("heading", { name: "Личные данные" })).toBeInTheDocument();
});

function box(top: number, height: number): DOMRect {
  return {
    top,
    bottom: top + height,
    left: 0,
    right: 200,
    width: 200,
    height,
    x: 0,
    y: top,
    toJSON() {
      return {};
    },
  } as DOMRect;
}

test("step_scrolls_an_offscreen_error", async () => {
  renderStep();
  const field = document.querySelector('[data-question="personalData.surname"]');
  if (!(field instanceof HTMLElement)) throw new Error("surname missing");
  vi.spyOn(field, "getBoundingClientRect").mockReturnValue(box(900, 80));
  const scroll = vi.spyOn(window, "scrollBy").mockImplementation(() => undefined);
  Object.defineProperty(window, "innerHeight", { configurable: true, value: 768 });
  fireEvent.click(screen.getByRole("button", { name: "Далее" }));
  await waitFor(() => expect(scroll).toHaveBeenCalled());
  expect(scroll).toHaveBeenCalledWith({ top: 980 - (768 - 16), behavior: "smooth" });
});

test("step_leaves_a_visible_error", async () => {
  renderStep();
  const field = document.querySelector('[data-question="personalData.surname"]');
  if (!(field instanceof HTMLElement)) throw new Error("surname missing");
  vi.spyOn(field, "getBoundingClientRect").mockReturnValue(box(40, 80));
  const scroll = vi.spyOn(window, "scrollBy").mockImplementation(() => undefined);
  Object.defineProperty(window, "innerHeight", { configurable: true, value: 768 });
  fireEvent.click(screen.getByRole("button", { name: "Далее" }));
  expect(await screen.findByText("Заполните это поле")).toBeInTheDocument();
  expect(scroll).not.toHaveBeenCalled();
});

test("step_locale_moves_the_screen", () => {
  const documents = foldForm();
  separate(documents);
  documents.conf[0].for_locales = ["ru"];
  documents.conf[0].advice = { ru: "Личные данные", en: "Personal data" };
  documents.conf[1].advice = { ru: "Ответьте на вопросы", en: "Answer the questions" };
  const state = build(documents, "ru");
  const view = render(<Step state={state} client={client} />);
  expect(screen.queryByLabelText("Язык")).not.toBeInTheDocument();
  expect(screen.getByRole("heading", { name: "Личные данные" })).toBeInTheDocument();
  setLocale(state, "en");
  view.rerender(<Step state={state} client={client} />);
  expect(screen.getByRole("button", { name: "Back" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Next" })).toBeInTheDocument();
  expect(screen.getByRole("heading", { name: "Answer the questions" })).toBeInTheDocument();
  expect(screen.queryByRole("heading", { name: "Личные данные" })).not.toBeInTheDocument();
  expect(screen.queryByLabelText("Фамилия")).not.toBeInTheDocument();
});
