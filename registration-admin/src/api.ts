export type LoginResult =
  | { ok: true; timeoutSecs: number }
  | { ok: false; code: string; timeoutSecs: number };

export type NetworkQuality = "good" | "fair" | "poor";

export type NetworkStatus = {
  samples: number;
  offline: boolean;
  quality: NetworkQuality | null;
  timeoutSecs: number;
  /** The page could not read `/api/network` from this machine. */
  localDown?: boolean;
};

type LoginResponse = {
  ok: boolean;
  timeout_secs?: number;
  data?: { timeout_secs?: number };
  error?: { code: string };
};

export async function fetchNetwork(): Promise<NetworkStatus> {
  const response = await fetch("/api/network", { credentials: "same-origin" });
  if (response.ok === false) {
    throw new Error("local server did not return network quality");
  }
  const body = (await response.json()) as {
    data: {
      samples: number;
      offline: boolean;
      quality: NetworkQuality | null;
      timeout_secs: number;
    };
  };
  return {
    samples: body.data.samples,
    offline: body.data.offline,
    quality: body.data.quality,
    timeoutSecs: body.data.timeout_secs,
  };
}

export async function postLogin(
  login: string,
  password: string,
): Promise<LoginResult> {
  const response = await fetch("/api/login", {
    method: "POST",
    headers: { "content-type": "application/json" },
    credentials: "same-origin",
    body: JSON.stringify({ login, password }),
  });
  const body = (await response.json()) as LoginResponse;
  const timeoutSecs = body.timeout_secs ?? body.data?.timeout_secs ?? 10;
  if (!body.ok) {
    return { ok: false, code: body.error?.code ?? "unknown_error", timeoutSecs };
  }
  return { ok: true, timeoutSecs };
}

export type Event = {
  id: string;
  name: string;
};

export type SelectResult =
  | { ok: true; id: string; name: string; tokenStored: boolean }
  | { ok: false; code: string };

export async function postSelect(id: string, name: string): Promise<SelectResult> {
  const response = await fetch("/api/events/select", {
    method: "POST",
    headers: { "content-type": "application/json" },
    credentials: "same-origin",
    body: JSON.stringify({ id, name }),
  });
  const body = (await response.json()) as {
    ok: boolean;
    data?: { id: string; name: string; token_stored: boolean };
    error?: { code: string };
  };
  if (!body.ok || !body.data) {
    return { ok: false, code: body.error?.code ?? "unknown_error" };
  }
  return {
    ok: true,
    id: body.data.id,
    name: body.data.name,
    tokenStored: body.data.token_stored,
  };
}

export async function fetchEvents(): Promise<
  | { ok: true; current: Event | null; events: Event[] }
  | { ok: false; code: string }
> {
  const response = await fetch("/api/events", { credentials: "same-origin" });
  const body = (await response.json()) as {
    ok: boolean;
    data?: { current: Event | null; events: Event[] };
    error?: { code: string };
  };
  if (!body.ok || !body.data) {
    return { ok: false, code: body.error?.code ?? "unknown_error" };
  }
  return {
    ok: true,
    current: body.data.current,
    events: body.data.events,
  };
}

export async function fetchBinding(): Promise<{ tokenStored: boolean }> {
  const response = await fetch("/api/binding", { credentials: "same-origin" });
  const body = (await response.json()) as {
    data?: { token_stored?: boolean };
  };
  return { tokenStored: body.data?.token_stored === true };
}

export async function postSession(): Promise<boolean> {
  const response = await fetch("/api/session", {
    method: "POST",
    credentials: "same-origin",
  });
  const body = (await response.json()) as { ok?: boolean };
  return body.ok === true;
}

export async function fetchKeys(): Promise<unknown[]> {
  const response = await fetch("/api/keys", { credentials: "same-origin" });
  const body = (await response.json()) as { data?: unknown[] };
  return body.data ?? [];
}
