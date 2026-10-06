import { createContext, useContext, useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";

import { StepFrameContext, type StepFrame } from "../layouts/frame";
import { PickerMenu, SearchSelect, pickerClass, type PickerRow } from "../picker";
import type { EnumParents, EnumResult, Screen } from "./client";
import { choiceOptions, choose } from "./choices";
import { flagSrc } from "./flags";
import { back, check, next, openScreens, readValue, setGrouped, setValue, type FormState, type Question, type StepError } from "./engine";
import { watchCities } from "./files";
import {
  loadPhoneCountries,
  matchPhoneCountries,
  openPhone,
  phoneCallingCode,
  phoneCountryId,
  phoneMask,
  phoneNational,
  phoneSlotCount,
  phoneText,
  setNational,
  setPhoneCountry,
  watchPhone,
  type PhoneCountry,
} from "./phones";
import { cityLabel, countryLabel, countryValue, openCitySearch, searchCities, searchCountries, setCity, setCountry } from "./place";

type StepClient = {
  counts(kind: "email" | "phone", value: string): Promise<{ count: number }>;
  enums?(locale: string, name: string, parents: EnumParents, options?: { refresh?: boolean }): Promise<EnumResult>;
};

const CITY_MATCHES = 12;

function screenMatches(screen: Screen, locale: string): boolean {
  return !screen.for_locales || screen.for_locales.length === 0 || screen.for_locales.includes(locale);
}

function adviceText(screen: Screen, locale: string): string {
  const text = locale === "en" ? screen.advice.en : screen.advice.ru;
  return text ?? "";
}

function labelOf(question: Question, locale: string): string {
  if (locale === "en" && question.label.en) return question.label.en;
  return question.label.ru ?? question.label.en ?? question.id;
}

function isRequired(question: Question, locale: string): boolean {
  return question.req || question.reqLoc[locale] === true;
}

function FieldLabel({ question, locale, htmlFor }: { question: Question; locale: string; htmlFor: string }) {
  const sheet = useContext(SheetContext);
  return (
    <label className="block" htmlFor={htmlFor}>
      {labelOf(question, locale)}
      {sheet || !isRequired(question, locale) ? null : (
        <span className="text-red-700" aria-hidden="true"> *</span>
      )}
    </label>
  );
}

function itemIds(item: unknown): string[] {
  const group = Array.isArray(item) ? item : [item];
  const ids: string[] = [];
  for (const source of group) {
    if (source && typeof source === "object" && "id" in source && typeof source.id === "string") {
      ids.push(source.id);
    }
  }
  return ids;
}

function columnClass(frame: StepFrame | undefined): string {
  if (frame === "ipad") return "mx-auto flex w-full max-w-kiosk flex-col items-center gap-4 pb-bar";
  if (frame === "web") return "mx-auto flex w-full max-w-web flex-col gap-4";
  if (frame === "phone") return "flex w-full flex-col gap-4 pb-bar";
  return "flex flex-col gap-4";
}

function rowClass(frame: StepFrame | undefined): string {
  if (frame === "ipad" || frame === "web") return "flex w-full flex-row gap-4";
  return "flex w-full flex-col gap-4";
}

const SheetContext = createContext(false);

const ERROR_GAP = 16;

function sheetWidth(question: Question): string {
  if (question.many || question.radios) return "mb-4 w-full px-2";
  return "box-border w-full px-2 min-[601px]:w-1/2 min-[865px]:w-1/3";
}

function scrollFieldIntoView(id: string) {
  const field = document.querySelector(`[data-question="${CSS.escape(id)}"]`);
  if (!(field instanceof HTMLElement)) return;
  const bar = document.querySelector('[data-testid="bar"]');
  const covered = bar instanceof HTMLElement && bar.classList.contains("fixed") ? bar.getBoundingClientRect().height : 0;
  const rect = field.getBoundingClientRect();
  const bottomEdge = window.innerHeight - covered;
  if (rect.top >= 0 && rect.bottom <= bottomEdge) return;
  const room = bottomEdge - ERROR_GAP * 2;
  const delta = rect.height > room || rect.top < 0 ? rect.top - ERROR_GAP : rect.bottom - (bottomEdge - ERROR_GAP);
  if (delta === 0) return;
  window.scrollBy({ top: delta, behavior: "smooth" });
}

function barClass(frame: StepFrame | undefined): string {
  if (frame === "web") return "mt-6 flex w-full gap-3";
  if (frame === "ipad" || frame === "phone") return "fixed inset-x-0 bottom-0 z-10 flex gap-3";
  return "flex gap-3";
}

function controlKind(question: Question): "place" | "phone" | "email" | "choice" | "text" {
  const leaf = question.leaf;
  if (
    leaf?.kind === "enum" &&
    (leaf.enumName === "country" || leaf.enumName === "region" || leaf.enumName === "city")
  ) {
    return "place";
  }
  if (/(?:contact_phones|phones_faxes|faxes)\.str_number$/.test(question.id)) return "phone";
  if (question.id.endsWith("emails.email")) return "email";
  if (leaf?.kind === "option") return "choice";
  return "text";
}

export function PlaceSelect({ id }: { id: string }) {
  return <select id={id} className="w-full" data-testid="place" />;
}

function countryQuestionId(cityId: string): string {
  return cityId.replace(/\.city_db$/, ".country_db");
}

function visibleCountryId(state: FormState): string | undefined {
  for (const index of state.step) {
    const screen = state.screens[index];
    if (!screen) continue;
    for (const item of screen.body) {
      for (const id of itemIds(item)) {
        const question = state.questions[id];
        if (!question || question.hidden) continue;
        if (question.leaf?.kind === "enum" && question.leaf.enumName === "country") return id;
      }
    }
  }
  return undefined;
}

function CountryField({
  state,
  client,
  question,
  onEdit,
}: {
  state: FormState;
  client: StepClient;
  question: Question;
  onEdit: () => void;
}) {
  const path = question.id.slice(0, question.id.lastIndexOf("."));
  const ref = useRef<HTMLInputElement>(null);
  const queryRef = useRef(countryLabel(state, path));
  const [query, setQuery] = useState(queryRef.current);
  const selected = countryValue(state, path);
  const named = countryLabel(state, path);
  const filtering = query.trim() !== "" && query !== named;
  const hits = searchCountries(state, path, filtering ? query : "");

  function show(next: string) {
    queryRef.current = next;
    setQuery(next);
  }

  useEffect(() => {
    if (!client.enums) return;
    let stopped = false;
    const enums = client.enums.bind(client);
    void openCitySearch(state, { enums }, path)
      .then(() => {
        if (stopped) return;
        const name = countryLabel(state, path);
        if (queryRef.current !== "" && queryRef.current !== name) return;
        show(name);
      })
      .catch(() => undefined);
    return () => {
      stopped = true;
    };
  }, [state, client, path, state.locale]);

  useLayoutEffect(() => {
    if (state.countryFocus !== question.id) return;
    state.countryFocus = undefined;
    ref.current?.focus();
  });

  async function pick(id: number, name: string) {
    if (!client.enums) throw new Error("enums");
    await setCountry(state, { enums: client.enums.bind(client) }, path, id);
    show(name);
    onEdit();
  }

  const rows: PickerRow[] = hits.map((hit) => ({
    id: String(hit.id),
    label: hit.name,
    selected: hit.id === selected,
  }));

  return (
    <SearchSelect
      id={question.id}
      testId="country"
      inputRef={ref}
      value={query}
      rows={rows}
      onChange={show}
      onBlur={() => show(countryLabel(state, path))}
      onPick={(id) => {
        const hit = hits.find((item) => String(item.id) === id);
        if (!hit) return;
        return pick(hit.id, hit.name);
      }}
    />
  );
}

function CitySearch({
  state,
  client,
  question,
  onEdit,
}: {
  state: FormState;
  client: StepClient;
  question: Question;
  onEdit: () => void;
}) {
  const { t } = useTranslation();
  const path = question.id.slice(0, question.id.lastIndexOf("."));
  const queryRef = useRef("");
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState(() => [] as ReturnType<typeof searchCities>);
  const selectedCountry = countryValue(state, path);
  const [seenCountry, setSeenCountry] = useState(selectedCountry);
  if (selectedCountry !== seenCountry) {
    setSeenCountry(selectedCountry);
    queryRef.current = "";
    setQuery("");
    setHits([]);
  }
  const country = countryLabel(state, path);
  const otherCountry = country ? t("place.otherCountry", { country, lng: state.locale }) : "";

  function show(next: string) {
    queryRef.current = next;
    setQuery(next);
    setHits(searchCities(state, path, next).slice(0, CITY_MATCHES));
  }

  function restore() {
    if (queryRef.current !== "") return;
    const name = cityLabel(state, path);
    if (name === "") return;
    queryRef.current = name;
    setQuery(name);
  }

  useEffect(() => {
    if (!client.enums) return;
    let stopped = false;
    const enums = client.enums.bind(client);
    void openCitySearch(state, { enums }, path)
      .then(() => {
        if (stopped) return;
        restore();
        setHits(searchCities(state, path, queryRef.current).slice(0, CITY_MATCHES));
      })
      .catch(() => undefined);
    const stop = watchCities(state, () => {
      if (stopped) return;
      restore();
      setHits(searchCities(state, path, queryRef.current).slice(0, CITY_MATCHES));
    });
    return () => {
      stopped = true;
      stop();
    };
  }, [state, client, path, state.locale]);

  async function pick(id: number, name: string) {
    if (!client.enums) throw new Error("enums");
    await setCity(state, { enums: client.enums.bind(client) }, path, id);
    show(name);
    onEdit();
  }

  function showCountryField() {
    const present = visibleCountryId(state);
    const countryId = countryQuestionId(question.id);
    if (!present) {
      if (!state.questions[countryId]) return;
      state.countryFields[countryId] = true;
      state.countryFocus = countryId;
    } else {
      state.countryFocus = present;
    }
    onEdit();
  }

  const rows: PickerRow[] = [
    ...(otherCountry ? [{ id: "other-country", label: otherCountry, tone: "notice" as const }] : []),
    ...hits.map((hit) => ({
      id: String(hit.id),
      label: hit.name,
      selected: hit.id === question.def,
    })),
  ];

  return (
    <SearchSelect
      id={question.id}
      testId="place"
      value={query}
      rows={rows}
      onFocus={() => setHits(searchCities(state, path, queryRef.current).slice(0, CITY_MATCHES))}
      onChange={(next) => {
        show(next);
        if (next.trim() === "") setValue(state, question.id, state.locale, null);
      }}
      onPick={(id) => {
        if (id === "other-country") {
          showCountryField();
          return;
        }
        const hit = hits.find((item) => String(item.id) === id);
        if (!hit) return;
        return pick(hit.id, hit.name);
      }}
    />
  );
}

const PHONE_MATCHES = 12;

function Flag({ countryId }: { countryId: number | null }) {
  const src = flagSrc(countryId);
  if (!src) return null;
  return <img src={src} alt="" className="h-4 w-6 shrink-0 object-cover" />;
}

function PhoneList({
  state,
  client,
  question,
  onEdit,
}: {
  state: FormState;
  client: StepClient;
  question: Question;
  onEdit: () => void;
}) {
  const address = question.id.replace(/\.(?:contact_phones|phones_faxes|faxes)\.str_number$/, "");
  const inputRef = useRef<HTMLInputElement>(null);
  const resumeRef = useRef(false);
  const editRef = useRef(onEdit);
  editRef.current = onEdit;
  const [focused, setFocused] = useState(false);
  const [searching, setSearching] = useState(false);
  const [query, setQuery] = useState("");
  const [countries, setCountries] = useState<PhoneCountry[]>([]);
  const code = phoneCallingCode(state, address);
  const mask = phoneMask(state, address);
  const national = phoneNational(state, address);
  const shown = phoneText(mask, code, national, focused);
  const matches = matchPhoneCountries(countries, query, state.locale, phoneCountryId(state, address)).slice(0, PHONE_MATCHES);

  useEffect(() => {
    const stop = watchPhone(state, () => editRef.current());
    if (client.enums) {
      const enums = client.enums.bind(client);
      void openPhone(state, { enums }, question.id).catch(() => undefined);
    }
    return stop;
  }, [state, client, question.id]);

  useLayoutEffect(() => {
    const input = inputRef.current;
    if (!input || searching) return;
    if (resumeRef.current) {
      resumeRef.current = false;
      input.focus();
    }
    if (!focused) return;
    const slot = input.value.indexOf("_");
    const caret = slot < 0 ? input.value.length : slot;
    input.setSelectionRange(caret, caret);
  }, [shown, focused, searching]);

  function writeDigits(raw: string) {
    const slots = phoneSlotCount(mask, code);
    const digits = raw.replace(/\D/g, "");
    setNational(state, address, slots > 0 ? digits.slice(0, slots) : digits);
    onEdit();
  }

  function digitCount(value: string, caret: number): number {
    return value.slice(0, caret).replace(/\D/g, "").length;
  }

  function onDigitsKey(event: KeyboardEvent<HTMLInputElement>) {
    if (event.key !== "Backspace" && event.key !== "Delete") return;
    event.preventDefault();
    const input = event.currentTarget;
    const start = input.selectionStart ?? input.value.length;
    const end = input.selectionEnd ?? start;
    const from = digitCount(input.value, start);
    const to = digitCount(input.value, end);
    let next = national;
    if (from !== to) next = national.slice(0, from) + national.slice(to);
    else if (event.key === "Backspace" && from > 0) next = national.slice(0, from - 1) + national.slice(from);
    else if (event.key === "Delete" && from < national.length) next = national.slice(0, from) + national.slice(from + 1);
    else return;
    setNational(state, address, next);
    onEdit();
  }

  async function pick(id: number) {
    if (!client.enums) return;
    try {
      await setPhoneCountry(state, { enums: client.enums.bind(client) }, address, id);
    } catch {
      return;
    }
    setNational(state, address, "");
    resumeRef.current = true;
    setSearching(false);
    setFocused(true);
    setQuery("");
    onEdit();
  }

  const countryId = phoneCountryId(state, address);
  const codeRows: PickerRow[] = matches.map((row) => ({
    id: String(row.id),
    label: `${row.name} ${row.code}`,
    selected: row.id === countryId,
    leading: <Flag countryId={row.id} />,
  }));

  return (
    <div className={pickerClass(searching, "row")}>
      <button
        type="button"
        className="inline-flex shrink-0 items-center gap-2 rounded-r-none border border-slate-300 bg-white px-3 text-gray-900"
        onMouseDown={(event) => {
          event.preventDefault();
          setFocused(false);
          setSearching(true);
          if (!client.enums || countries.length > 0) return;
          const enums = client.enums.bind(client);
          void loadPhoneCountries(state, { enums })
            .then((rows) => setCountries(rows))
            .catch(() => undefined);
        }}
      >
        <Flag countryId={phoneCountryId(state, address)} />
        {code}
      </button>
      {searching ? (
        <input
          id={question.id}
          ref={inputRef}
          className="min-w-0 w-full flex-1 rounded-l-none border-l-0"
          data-testid="phone-search"
          value={query}
          autoFocus
          onChange={(event) => setQuery(event.target.value)}
          onBlur={() => setSearching(false)}
        />
      ) : (
        <input
          id={question.id}
          ref={inputRef}
          className="min-w-0 w-full flex-1 rounded-l-none border-l-0"
          data-testid="phone"
          inputMode="tel"
          autoComplete="off"
          placeholder={focused ? phoneText(mask, code, "", true) : ""}
          value={shown}
          onFocus={() => setFocused(true)}
          onBlur={() => setFocused(false)}
          onKeyDown={onDigitsKey}
          onChange={(event) => writeDigits(event.target.value)}
        />
      )}
      {searching ? <PickerMenu rows={codeRows} onPick={(id) => void pick(Number(id))} /> : null}
    </div>
  );
}

export function EmailList({ id, value, onValue }: { id: string; value: string; onValue: (value: string) => void }) {
  return (
    <input
      id={id}
      type="email"
      className="w-full"
      data-testid="email"
      value={value}
      onChange={(event) => onValue(event.target.value)}
    />
  );
}

export function ChoiceList({
  state,
  question,
  onEdit,
}: {
  state: FormState;
  question: Question;
  onEdit: () => void;
}) {
  const sheet = useContext(SheetContext);
  const options = choiceOptions(state, question.id, state.locale);
  const selected = readValue(state, question.id, state.locale);
  if (question.many || question.radios) {
    const picked = Array.isArray(selected) ? selected : [];
    return (
      <div
        className={sheet ? "grid w-full grid-cols-1 gap-x-3 gap-y-1 min-[601px]:grid-cols-2 min-[865px]:grid-cols-3" : "w-full"}
        data-testid="choice"
        role={question.radios ? "radiogroup" : undefined}
      >
        {options.map((option) => (
          <label
            key={option.id}
            className={sheet ? "flex items-start gap-1 text-sm" : "block"}
            style={{ paddingLeft: option.depth * (sheet ? 12 : 16) }}
          >
            <input
              type={question.radios ? "radio" : "checkbox"}
              name={question.radios ? question.id : undefined}
              checked={question.radios ? selected === option.id : picked.includes(option.id)}
              onChange={() => {
                choose(state, question.id, option.id);
                onEdit();
              }}
            />
            {option.label}
          </label>
        ))}
      </div>
    );
  }
  return (
    <select
      id={question.id}
      className="w-full"
      data-testid="choice"
      value={typeof selected === "number" ? String(selected) : ""}
      onChange={(event) => {
        if (event.target.value === "") return;
        choose(state, question.id, Number(event.target.value));
        onEdit();
      }}
    >
      <option value="" />
      {options.map((option) => (
        <option key={option.id} value={option.id}>
          {option.label}
        </option>
      ))}
    </select>
  );
}

function QuestionControl({
  state,
  client,
  question,
  error,
  onEdit,
}: {
  state: FormState;
  client: StepClient;
  question: Question;
  error?: StepError;
  onEdit: () => void;
}) {
  const { t } = useTranslation();
  const frame = useContext(StepFrameContext);
  const kind = controlKind(question);
  const current = readValue(state, question.id, state.locale);
  const text = typeof current === "string" ? current : "";
  const write = (value: string) => {
    setValue(state, question.id, state.locale, value);
    onEdit();
  };
  const grow = frame === "ipad" || frame === "web" ? "min-w-0 flex-1" : "w-full";
  const countryId = countryQuestionId(question.id);
  const countryQuestion = state.questions[countryId];
  const showCountry =
    question.leaf?.enumName === "city" &&
    state.countryFields[countryId] === true &&
    visibleCountryId(state) === undefined &&
    countryQuestion !== undefined;
  const field = (
    <div data-question={question.id} className={showCountry ? "w-full" : grow}>
      <FieldLabel question={question} locale={state.locale} htmlFor={question.id} />
      {kind === "place" && question.leaf?.enumName === "city" ? (
        <CitySearch state={state} client={client} question={question} onEdit={onEdit} />
      ) : null}
      {kind === "place" && question.leaf?.enumName === "country" ? (
        <CountryField state={state} client={client} question={question} onEdit={onEdit} />
      ) : null}
      {kind === "place" && question.leaf?.enumName === "region" ? <PlaceSelect id={question.id} /> : null}
      {kind === "phone" ? <PhoneList state={state} client={client} question={question} onEdit={onEdit} /> : null}
      {kind === "email" ? <EmailList id={question.id} value={text} onValue={write} /> : null}
      {kind === "choice" ? <ChoiceList state={state} question={question} onEdit={onEdit} /> : null}
      {kind === "text" ? (
        <input id={question.id} className="w-full" value={text} onChange={(event) => write(event.target.value)} />
      ) : null}
      {error ? <p className="text-red-700">{t(`error.${error.reason}`, { lng: state.locale })}</p> : null}
    </div>
  );
  if (!showCountry || !countryQuestion) return field;
  return (
    <div className="flex w-full flex-col gap-4">
      <div data-question={countryQuestion.id} className="w-full">
        <FieldLabel question={countryQuestion} locale={state.locale} htmlFor={countryQuestion.id} />
        <CountryField state={state} client={client} question={countryQuestion} onEdit={onEdit} />
      </div>
      {field}
    </div>
  );
}

function BodyItem({
  state,
  client,
  item,
  error,
  onEdit,
}: {
  state: FormState;
  client: StepClient;
  item: unknown;
  error?: StepError;
  onEdit: () => void;
}) {
  const frame = useContext(StepFrameContext);
  const sheet = useContext(SheetContext);
  const visible = itemIds(item)
    .map((id) => state.questions[id])
    .filter((question): question is Question => question !== undefined && !question.hidden);
  if (visible.length === 0) return null;
  const controls = visible.map((question) => (
    <QuestionControl
      key={question.id}
      state={state}
      client={client}
      question={question}
      error={error?.id === question.id ? error : undefined}
      onEdit={onEdit}
    />
  ));
  if (sheet) {
    return (
      <>
        {visible.map((question) => (
          <div key={question.id} data-testid="body-row" className={sheetWidth(question)}>
            <QuestionControl
              state={state}
              client={client}
              question={question}
              error={error?.id === question.id ? error : undefined}
              onEdit={onEdit}
            />
          </div>
        ))}
      </>
    );
  }
  if (Array.isArray(item)) {
    return (
      <div role="group" data-testid="body-row" className={rowClass(frame)}>
        {controls}
      </div>
    );
  }
  if (!frame) return <>{controls}</>;
  return (
    <div data-testid="body-row" className={rowClass(frame)}>
      {controls}
    </div>
  );
}

export function Step({
  state,
  client,
  onChange,
  onEdit,
  onBack,
  sheet = false,
}: {
  state: FormState;
  client: StepClient;
  onChange?: () => void;
  onEdit?: () => void;
  onBack?: () => void;
  sheet?: boolean;
}) {
  const { t } = useTranslation();
  const frame = useContext(StepFrameContext);
  const [, setTick] = useState(0);
  const [scrollRequest, setScrollRequest] = useState(0);
  const refresh = () => setTick((value) => value + 1);
  const shown = () => (sheet ? openScreens(state) : state.step);
  const edited = () => {
    const previous = state.errors[0]?.id;
    if (state.errors.length > 0) state.errors = check(state, shown(), state.locale);
    refresh();
    onEdit?.();
    const nextId = state.errors[0]?.id;
    if (nextId && nextId !== previous) setScrollRequest((value) => value + 1);
  };
  const error = state.errors[0];
  useEffect(() => {
    const id = state.errors[0]?.id;
    if (!id || scrollRequest === 0) return;
    scrollFieldIntoView(id);
  }, [scrollRequest, state]);
  useLayoutEffect(() => {
    if (sheet || frame === undefined) return;
    setGrouped(state, false);
    refresh();
  }, [frame, state, sheet]);

  return (
    <SheetContext.Provider value={sheet}>
    <section className={sheet ? "flex w-full flex-col gap-3" : columnClass(frame)} data-testid="step">
      {shown().map((index) => {
        const screen = state.screens[index];
        if (!screen || !screenMatches(screen, state.locale)) return null;
        const advice = adviceText(screen, state.locale);
        return (
          <div key={index} className={sheet ? "flex w-full flex-wrap" : "flex w-full flex-col gap-4"}>
            {advice ? <h2 className={sheet ? "w-full px-2" : undefined}>{advice}</h2> : null}
            {screen.body.map((item, itemIndex) => (
              <BodyItem key={itemIndex} state={state} client={client} item={item} error={error} onEdit={edited} />
            ))}
          </div>
        );
      })}
      {sheet ? null : <div data-testid="bar" className={barClass(frame)}>
        <button
          type="button"
        onClick={() => {
          if (!back(state)) onBack?.();
          refresh();
          onChange?.();
        }}
        >
          {t("action.back", { lng: state.locale })}
        </button>
        <button
          type="button"
        onClick={() => {
          void next(state, client).finally(() => {
            refresh();
            onChange?.();
            if (state.errors.length > 0) setScrollRequest((value) => value + 1);
          });
        }}
        >
          {t("action.next", { lng: state.locale })}
        </button>
      </div>}
    </section>
    </SheetContext.Provider>
  );
}
