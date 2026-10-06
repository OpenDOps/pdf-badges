# Pickers

Category, printer, country, city, and the phone calling code share one list. The field in front of that list is one of two controls. Both live in `registration-form/app/picker.tsx`.

## ClosedSelect

Use this when the person picks one row from a short list and the field does not accept typing. Category and printer on the form, and the printer in the visitor-list header, are this control. The visitor-list category filter is `MultiSelect`.

| prop | |
|---|---|
| `label` | Accessible name of the field |
| `options` | `{ id, label }[]` |
| `value` | Selected id. An id missing from `options` shows an empty field |
| `onPick` | Called with the chosen id. The list then closes |

A click opens the list. A click on the same field closes it. A click opens it again when the field is already focused and the list is closed. Escape closes it. The current row is highlighted. The field is read-only.

```ts
<ClosedSelect
  label="Категория"
  options={categories}
  value={categoryId}
  onPick={(id) => setCategoryId(id)}
/>
```

The list is drawn inside the field, so a parent `label` would also activate the input. `PickerMenu` cancels that click. Keep the visible caption in a `span` beside `ClosedSelect`, or in a `label` that wraps both. The cancel covers the wrapped case.

## MultiSelect

Use this when several rows can be on at once and the field does not accept typing. The visitor-list category filter is this control. `none` is the id that means the whole list, `""` for "Все категории".

| prop | |
|---|---|
| `label` | Accessible name of the field |
| `options` | `{ id, label }[]`, with the `none` row included |
| `value` | Selected ids. An empty list shows the `none` label |
| `none` | The id that stands alone. Choosing it clears every other id |
| `onChange` | The next id list. Choosing `none` closes the menu. Any other row leaves it open |

Choosing any other id adds it, and clears `none`. Choosing that id again removes it. When the last id is removed, the field returns to `none`. Selected rows stay highlighted. Choosing `none` clears the others and closes the list. A click on the field, Escape, or blur also closes it.

```ts
<MultiSelect
  label="Категория"
  options={categories}
  value={categoryIds}
  none=""
  onChange={setCategoryIds}
/>
```

## SearchSelect

Use this when the person types to narrow a long catalog. Country and city are this control. The caller owns the query string and the rows. `SearchSelect` owns opening and closing.

| prop | |
|---|---|
| `id` | Input id, so a `label` with `htmlFor` names the field |
| `testId` | Optional `data-testid` |
| `inputRef` | Optional ref, when another control must focus this field |
| `value` | Text currently in the field |
| `rows` | Rows to show while the list is open. See `PickerRow` |
| `onChange` | The typed text |
| `onPick` | Chosen row id. A returned promise that rejects leaves the list open |
| `onFocus` | Optional, before the list opens |
| `onBlur` | Optional, after the list closes. Country writes the selected name back here |

Focus opens the list. Typing opens it and calls `onChange`. Escape closes it. Enter chooses the first row whose `tone` is not `"notice"`. Blur closes it.

```ts
<SearchSelect
  id={question.id}
  value={query}
  rows={hits.map((hit) => ({ id: String(hit.id), label: hit.name, selected: hit.id === chosen }))}
  onChange={setQuery}
  onPick={(id) => save(Number(id))}
/>
```

City puts one extra row first: `{ id: "other-country", label, tone: "notice" }`. That row is amber. Enter skips it and chooses the first city.

## PickerMenu

The list these controls draw. A row is `{ id, label, selected?, tone?, leading? }`. `selected` highlights the current row. `tone: "notice"` is the amber row. `leading` is a node before the label, such as a flag.

Use `PickerMenu` alone when the search is not the whole field. The phone calling code is that case: a button with the flag and the code replaces the number with a search input, and `PickerMenu` hangs under that row. `pickerClass(open, "row")` is the shared frame for that row. `pickerClass(open)` is the frame for a single field.

A press on a row uses `mousedown` to keep focus on the field, then `click` to choose. Choosing on `mousedown` would close the list before the click, and the click would land on the field and open the list again.

## What does not use these

`ChoiceList` is the questionnaire control: radios, checkboxes, or a native `select`. `PlaceSelect` is the region field, a native `select` with no rows yet. The phone number itself stays in `PhoneList`; only its country list is `PickerMenu`.
