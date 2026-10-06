import { useState, type ReactNode, type Ref } from "react";

export type PickerOption = { id: string; label: string };

export type PickerRow = PickerOption & {
  selected?: boolean;
  tone?: "notice";
  leading?: ReactNode;
};

export function pickerClass(open: boolean, layout: "field" | "row" = "field"): string {
  const layer = open ? "relative z-50" : "relative z-30";
  return layout === "row" ? `${layer} flex w-full` : `${layer} w-full`;
}

function rowClass(row: PickerRow): string {
  const tone =
    row.tone === "notice"
      ? "cursor-pointer bg-amber-100 px-3 py-2 text-left font-semibold text-gray-900 hover:bg-amber-200"
      : row.selected
        ? "cursor-pointer bg-sky-100 px-3 py-2 text-left font-semibold text-gray-900 hover:bg-sky-200"
        : "cursor-pointer bg-white px-3 py-2 text-left text-gray-900 hover:bg-sky-100";
  return row.leading ? `flex items-center gap-2 ${tone}` : tone;
}

export function PickerMenu({
  rows,
  onPick,
  multiple = false,
}: {
  rows: PickerRow[];
  onPick: (id: string) => void;
  multiple?: boolean;
}) {
  if (rows.length === 0) return null;
  return (
    <ul
      role="listbox"
      aria-multiselectable={multiple || undefined}
      className="absolute top-full z-20 mt-1 max-h-60 w-full overflow-auto rounded-md border border-slate-300 bg-white shadow"
    >
      {rows.map((row) => (
        <li
          key={row.id}
          role="option"
          aria-selected={row.selected === true}
          className={rowClass(row)}
          onMouseDown={(event) => event.preventDefault()}
          onClick={(event) => {
            event.preventDefault();
            onPick(row.id);
          }}
        >
          {row.leading}
          {row.label}
        </li>
      ))}
    </ul>
  );
}

export function ClosedSelect({
  label,
  options,
  value,
  onPick,
}: {
  label: string;
  options: PickerOption[];
  value: string;
  onPick: (id: string) => void;
}) {
  const named = options.find((item) => item.id === value)?.label ?? "";
  const [open, setOpen] = useState(false);

  function pick(id: string) {
    onPick(id);
    setOpen(false);
  }

  return (
    <div className={pickerClass(open)}>
      <input
        className="w-full cursor-pointer caret-transparent"
        role="combobox"
        aria-label={label}
        aria-expanded={open}
        aria-autocomplete="none"
        readOnly
        value={named}
        onClick={() => setOpen((current) => !current)}
        onBlur={() => setOpen(false)}
        onKeyDown={(event) => {
          if (event.key === "Escape") setOpen(false);
        }}
      />
      {open ? (
        <PickerMenu
          rows={options.map((item) => ({ ...item, selected: item.id === value }))}
          onPick={pick}
        />
      ) : null}
    </div>
  );
}

export function MultiSelect({
  label,
  options,
  value,
  none,
  onChange,
}: {
  label: string;
  options: PickerOption[];
  value: string[];
  none: string;
  onChange: (ids: string[]) => void;
}) {
  const chosen = new Set(value);
  const summary = options
    .filter((item) => item.id !== none && chosen.has(item.id))
    .map((item) => item.label)
    .join(", ");
  const shown = summary || options.find((item) => item.id === none)?.label || "";
  const [open, setOpen] = useState(false);

  function toggle(id: string) {
    if (id === none) {
      onChange([]);
      setOpen(false);
      return;
    }
    onChange(chosen.has(id) ? value.filter((item) => item !== id) : [...value, id]);
  }

  return (
    <div className={pickerClass(open)}>
      <input
        className="w-full cursor-pointer truncate caret-transparent"
        role="combobox"
        aria-label={label}
        aria-expanded={open}
        aria-autocomplete="none"
        readOnly
        value={shown}
        onClick={() => setOpen((current) => !current)}
        onBlur={() => setOpen(false)}
        onKeyDown={(event) => {
          if (event.key === "Escape") setOpen(false);
        }}
      />
      {open ? (
        <PickerMenu
          multiple
          rows={options.map((item) => ({
            ...item,
            selected: item.id === none ? value.length === 0 : chosen.has(item.id),
          }))}
          onPick={toggle}
        />
      ) : null}
    </div>
  );
}

export function SearchSelect({
  id,
  testId,
  inputRef,
  value,
  rows,
  onChange,
  onPick,
  onFocus,
  onBlur,
}: {
  id: string;
  testId?: string;
  inputRef?: Ref<HTMLInputElement>;
  value: string;
  rows: PickerRow[];
  onChange: (value: string) => void;
  onPick: (id: string) => void | Promise<void>;
  onFocus?: () => void;
  onBlur?: () => void;
}) {
  const [open, setOpen] = useState(false);

  function choose(id: string) {
    let result: void | Promise<void>;
    try {
      result = onPick(id);
    } catch {
      setOpen(true);
      return;
    }
    if (result instanceof Promise) {
      void result.then(() => setOpen(false)).catch(() => setOpen(true));
      return;
    }
    setOpen(false);
  }

  return (
    <div className={pickerClass(open)}>
      <input
        ref={inputRef}
        id={id}
        className="w-full"
        data-testid={testId}
        role="combobox"
        aria-expanded={open}
        autoComplete="off"
        value={value}
        onFocus={() => {
          onFocus?.();
          setOpen(true);
        }}
        onBlur={() => {
          setOpen(false);
          onBlur?.();
        }}
        onChange={(event) => {
          onChange(event.target.value);
          setOpen(true);
        }}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            setOpen(false);
            return;
          }
          const choice = rows.find((row) => row.tone !== "notice");
          if (event.key !== "Enter" || !choice) return;
          event.preventDefault();
          choose(choice.id);
        }}
      />
      {open ? <PickerMenu rows={rows} onPick={choose} /> : null}
    </div>
  );
}
