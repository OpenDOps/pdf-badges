import { useState } from "react";
import { act, fireEvent, render, screen } from "@testing-library/react";

import { fetchKeys, postSelect } from "./api";
import "./i18n";
import Events, { type Event } from "./Events";

const events: Event[] = [
  { id: "5245081", name: "TechCrunch" },
  { id: "2", name: "Bo" },
];

function renderEvents(
  props: Partial<{
    events: Event[];
    current: Event | null;
    tokenStored: boolean;
    onSelect: (id: string, name: string) => void;
    watchNetwork: boolean;
    keepSession: boolean;
    timeoutSecs: number;
  }> = {},
) {
  return render(
    <Events
      events={props.events ?? events}
      current={props.current === undefined ? events[0] : props.current}
      tokenStored={props.tokenStored ?? false}
      onSelect={props.onSelect ?? (() => {})}
      watchNetwork={props.watchNetwork ?? false}
      keepSession={props.keepSession ?? false}
      timeoutSecs={props.timeoutSecs ?? 0}
    />,
  );
}

test("events_lists_names", () => {
  renderEvents();
  expect(screen.getByRole("button", { name: "TechCrunch" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Bo" })).toBeInTheDocument();
  expect(
    screen.getByRole("region", { name: "Текущее мероприятие" }),
  ).toHaveTextContent("TechCrunch");
});

test("events_selects_one", () => {
  const onSelect = vi.fn();
  renderEvents({ onSelect });
  fireEvent.click(screen.getByRole("button", { name: "TechCrunch" }));
  expect(onSelect).toHaveBeenCalledTimes(1);
  expect(onSelect).toHaveBeenCalledWith("5245081", "TechCrunch");
});

test("token_stored_line", () => {
  const { rerender } = renderEvents({ tokenStored: true });
  const stored = screen.getByRole("status");
  expect(stored).toHaveTextContent("Токен проекта сохранён");
  expect(stored).not.toHaveTextContent("token-value");

  rerender(
    <Events
      events={events}
      current={events[0]}
      tokenStored={false}
      onSelect={() => {}}
      watchNetwork={false}
      keepSession={false}
      timeoutSecs={0}
    />,
  );
  expect(screen.getByRole("status")).toHaveTextContent(
    "Токен проекта отсутствует",
  );
});

test("events_shows_the_corner_signal", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue({
      json: async () => ({
        ok: true,
        data: { samples: 2, offline: false, quality: "fair", timeout_secs: 6 },
      }),
    }),
  );
  renderEvents({ watchNetwork: true });
  const signal = await screen.findByText("Связь средняя");
  expect(signal).toHaveAttribute("data-signal", "warn");
  expect(signal).toHaveClass("fixed", "top-4", "right-4");
  expect(screen.getByText("Токен проекта отсутствует")).toBeInTheDocument();
  vi.unstubAllGlobals();
});

test("events_pings_the_session", async () => {
  vi.useFakeTimers();
  const fetchMock = vi.fn().mockResolvedValue({
    json: async () => ({ ok: true, data: {} }),
  });
  vi.stubGlobal("fetch", fetchMock);
  renderEvents({ keepSession: true, watchNetwork: false });
  await act(async () => {
    await Promise.resolve();
  });
  expect(fetchMock).toHaveBeenCalledWith(
    "/api/session",
    expect.objectContaining({ method: "POST" }),
  );
  const calls = fetchMock.mock.calls.length;
  await act(async () => {
    await vi.advanceTimersByTimeAsync(30_000);
  });
  expect(fetchMock.mock.calls.length).toBeGreaterThan(calls);
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

test("select_counts_down_and_disables_choices", async () => {
  vi.useFakeTimers();
  const onSelect = vi.fn().mockResolvedValue(undefined);
  renderEvents({ onSelect, timeoutSecs: 3 });
  const chosen = screen.getByRole("button", { name: "TechCrunch" });
  act(() => {
    fireEvent.click(chosen);
  });
  expect(chosen).toBeDisabled();
  expect(screen.getByRole("button", { name: "Bo" })).toBeDisabled();
  expect(screen.getByRole("progressbar")).toHaveTextContent("Подождите 3 с");
  await act(async () => {
    await vi.advanceTimersByTimeAsync(1000);
  });
  expect(screen.getByRole("progressbar")).toHaveTextContent("Подождите 2 с");
  await act(async () => {
    await vi.advanceTimersByTimeAsync(2000);
  });
  expect(screen.queryByRole("progressbar")).not.toBeInTheDocument();
  expect(chosen).toBeEnabled();
  expect(onSelect).toHaveBeenCalledTimes(1);
  vi.useRealTimers();
});

test("select_shows_already_in_progress", async () => {
  renderEvents({
    timeoutSecs: 0,
    onSelect: async () => "select_in_progress",
  });
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "TechCrunch" }));
  });
  expect(screen.getByRole("alert")).toHaveTextContent(
    "Выбор мероприятия уже выполняется",
  );
});

test("select_shows_the_remote_error", async () => {
  renderEvents({
    timeoutSecs: 0,
    onSelect: async () => "no_connection",
  });
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "TechCrunch" }));
  });
  expect(screen.getByRole("alert")).toHaveTextContent(
    "Нет соединения с интернет-сервером",
  );
});

test("screen_shows_stored_and_empty_keys", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn(async (url: string) => {
      if (url.endsWith("/api/events/select")) {
        return {
          json: async () => ({
            ok: true,
            data: { id: "5245081", name: "TechCrunch", token_stored: true },
          }),
        };
      }
      if (url.endsWith("/api/keys")) {
        return { json: async () => ({ ok: true, data: [] }) };
      }
      throw new Error(url);
    }),
  );
  function Harness() {
    const [tokenStored, setTokenStored] = useState(false);
    return (
      <Events
        events={events}
        current={events[0]}
        tokenStored={tokenStored}
        watchNetwork={false}
        keepSession={false}
        timeoutSecs={0}
        onSelect={async (id, name) => {
          const selected = await postSelect(id, name);
          const keys = await fetchKeys();
          if (selected.ok && selected.tokenStored && keys.length === 0) {
            setTokenStored(true);
          }
        }}
      />
    );
  }
  render(<Harness />);
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "TechCrunch" }));
  });
  expect(screen.getByText("Токен проекта сохранён")).toBeInTheDocument();
  expect(screen.getByText("Нет ключей")).toBeInTheDocument();
  vi.unstubAllGlobals();
});

test("keys_block_empty", () => {
  renderEvents();
  const keys = screen.getByRole("region", { name: "Ключи" });
  expect(keys).toHaveTextContent("Ключи");
  expect(keys).toHaveTextContent("Нет ключей");
  expect(keys.querySelector("button")).toBeNull();
});
