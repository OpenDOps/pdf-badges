export type EnumParents = {
  country?: number | string;
  region?: number | string;
};

export type EnumRow = {
  id: number;
  v: string | { ru?: string; en?: string };
  d?: boolean;
  weight?: number;
};

export type EnumResult = {
  list: EnumRow[];
  error: Error | null;
};

export type Screen = {
  advice: { ru?: string; en?: string };
  body: unknown[];
  next: unknown[];
  for_locales?: string[];
  screen_block?: number;
  payment?: boolean;
};

export type FormDocuments = {
  vars: Record<string, unknown>;
  struct: unknown;
  model: { uniqueId: string } & Record<string, unknown>;
  conf: Screen[];
  hiddenq: unknown[];
  settings: unknown;
  barcodes: unknown;
};

const FORM = "/forms/form.json";

type StatusError = Error & { status?: number; code?: string };

let onUnauthorized: (() => void) | undefined;

export function setUnauthorized(handler: (() => void) | undefined) {
  onUnauthorized = handler;
}

export function isUnauthorized(error: unknown): boolean {
  return error instanceof Error && (error as StatusError).status === 401;
}

function unauthorized(path: string): StatusError {
  onUnauthorized?.();
  const route = path.split("?")[0] ?? path;
  const error = new Error(`${route} 401`) as StatusError;
  error.status = 401;
  return error;
}

function errorCode(body: unknown): string | undefined {
  if (!body || typeof body !== "object") return undefined;
  const error = (body as { error?: unknown }).error;
  if (!error || typeof error !== "object") return undefined;
  const code = (error as { code?: unknown }).code;
  return typeof code === "string" ? code : undefined;
}

function enumLocale(locale: string): "ru" | "en" {
  if (locale === "ru" || locale === "en") return locale;
  throw new Error("invalid enum locale");
}

function enumId(value: number | string | undefined): number {
  if (typeof value === "number" && Number.isInteger(value) && value >= 0) return value;
  throw new Error("invalid enum id");
}

export function enumPath(locale: string, name: string, parents: EnumParents): string {
  const lang = enumLocale(locale);
  if (name === "country") return `/dbenums/country_${lang}.json`;
  if (name === "region") return `/dbenums/country-${enumId(parents.country)}/region_${lang}.json`;
  if (name === "city") {
    return `/dbenums/country-${enumId(parents.country)}/region-${enumId(parents.region)}/city_${lang}.json`;
  }
  if (name === "phone_code") return "/dbenums/country_phone_code.json";
  if (name === "phone_mask") return "/dbenums/country_phone_mask.json";
  if (name === "phone_carrier") return "/dbenums/country_phone_carrier.json";
  if (name === "city_phone_mask") {
    return `/dbenums/country-${enumId(parents.country)}/region-${enumId(parents.region)}/city_phone_mask.json`;
  }
  throw new Error(`unknown enum ${name}`);
}

function enumCacheKey(locale: string, name: string, parents: EnumParents): string {
  const ids = Object.keys(parents)
    .sort()
    .map((key) => `${key}=${parents[key as keyof EnumParents]}`)
    .join("&");
  return `${name}|${locale}|${ids}`;
}

function unwrapData<T>(body: unknown): T {
  if (
    body &&
    typeof body === "object" &&
    !Array.isArray(body) &&
    "ok" in body &&
    (body as { ok: unknown }).ok === true &&
    "data" in body
  ) {
    return (body as { data: T }).data;
  }
  return body as T;
}

export class FormClient {
  private readonly cache = new Map<string, EnumRow[]>();
  private readonly inflight = new Map<string, Promise<EnumResult>>();

  private readonly fetchImpl: typeof fetch;

  constructor(fetchImpl: typeof fetch = fetch) {
    this.fetchImpl = (input, init) => fetchImpl.call(globalThis, input, init);
  }

  load(): Promise<FormDocuments> {
    return this.getJson<FormDocuments>(FORM);
  }

  static fromRequest(request: Request): FormClient {
    return new FormClient((input, init) => {
      const path = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
      const headers = new Headers(init?.headers);
      const cookie = request.headers.get("cookie");
      if (cookie && !headers.has("cookie")) headers.set("cookie", cookie);
      return fetch(new URL(path, request.url), { ...init, headers });
    });
  }

  async enums(
    locale: string,
    name: string,
    parents: EnumParents,
    options?: { refresh?: boolean },
  ): Promise<EnumResult> {
    const key = enumCacheKey(locale, name, parents);
    const pending = this.inflight.get(key);
    if (pending) return pending;
    if (!options?.refresh) {
      const cached = this.cache.get(key);
      if (cached) return { list: cached.slice(), error: null };
    }

    const path = enumPath(locale, name, parents);
    const task = this.loadEnum(key, path);
    this.inflight.set(key, task);
    try {
      return await task;
    } finally {
      if (this.inflight.get(key) === task) this.inflight.delete(key);
    }
  }

  private async loadEnum(key: string, path: string): Promise<EnumResult> {
    try {
      const list = await this.getJson<EnumRow[]>(path);
      this.cache.set(key, list);
      return { list: list.slice(), error: null };
    } catch (caught) {
      const error = caught instanceof Error ? caught : new Error(String(caught));
      const cached = this.cache.get(key);
      if (cached) return { list: cached.slice(), error };
      throw error;
    }
  }

  counts(kind: "email" | "phone", value: string): Promise<{ count: number }> {
    const query = new URLSearchParams({ [kind]: value });
    return this.getJson(`/api/registrations/counts?${query}`);
  }

  search(text: string): Promise<unknown> {
    const query = new URLSearchParams({ q: text });
    return this.getJson(`/api/registrations?${query}`);
  }

  registration(id: string): Promise<unknown> {
    return this.getJson(`/api/registrations/${encodeURIComponent(id)}`);
  }

  registrations(query: { q?: string; category?: string; limit: number; offset: number }): Promise<unknown> {
    const params = new URLSearchParams();
    if (query.q) params.set("q", query.q);
    if (query.category) params.set("category", query.category);
    params.set("limit", String(query.limit));
    params.set("offset", String(query.offset));
    return this.getJson(`/api/registrations?${params}`);
  }

  sync(): Promise<{ waiting: number; synced?: number; addresses?: string[]; is_admin?: boolean }> {
    return this.getJson("/api/sync");
  }

  network(): Promise<{
    samples: number;
    offline: boolean;
    quality: "good" | "fair" | "poor" | null;
    timeout_secs: number;
  }> {
    return this.getJson("/api/network");
  }

  async auth(key: string): Promise<void> {
    const path = "/api/desk/auth";
    const response = await this.fetchImpl(path, {
      method: "POST",
      credentials: "include",
      body: JSON.stringify({ key }),
      headers: { "Content-Type": "application/json" },
    });
    if (response.ok) return;
    let body: unknown = null;
    try {
      body = await response.json();
    } catch {
      body = null;
    }
    const code = errorCode(body);
    const error = new Error(code ?? `${path} ${response.status}`) as StatusError;
    error.status = response.status;
    error.code = code;
    throw error;
  }

  async save(visitor: unknown, printer?: string): Promise<unknown> {
    const path = "/api/registrations";
    const response = await this.fetchImpl(path, {
      method: "POST",
      credentials: "include",
      body: JSON.stringify({ visitor, printer }),
      headers: { "Content-Type": "application/json" },
    });
    if (response.status === 401) throw unauthorized(path);
    let body: unknown = null;
    try {
      body = await response.json();
    } catch {
      body = null;
    }
    if (!response.ok) {
      const code = errorCode(body);
      const error = new Error(code ?? `${path} ${response.status}`) as Error & { status?: number; code?: string };
      error.status = response.status;
      error.code = code;
      throw error;
    }
    return body;
  }

  async print(ids: string[], printer?: string): Promise<unknown> {
    const path = "/api/print";
    const response = await this.fetchImpl(path, {
      method: "POST",
      credentials: "include",
      body: JSON.stringify({ ids, printer }),
      headers: { "Content-Type": "application/json" },
    });
    if (response.status === 401) throw unauthorized(path);
    let body: unknown = null;
    try {
      body = await response.json();
    } catch {
      body = null;
    }
    if (!response.ok) {
      const code = errorCode(body);
      const error = new Error(code ?? `${path} ${response.status}`) as Error & { status?: number; code?: string };
      error.status = response.status;
      error.code = code;
      throw error;
    }
    return body;
  }

    photo(id: string, bytes: Blob): Promise<unknown> {
    return this.send(`/api/registrations/${encodeURIComponent(id)}/photo`, bytes);
  }

  printers(): Promise<unknown> {
    return this.getJson("/api/printers");
  }

  categories(): Promise<unknown> {
    return this.getJson("/api/forms/categories");
  }

  async catalog(
    rev: number,
    signal?: AbortSignal,
  ): Promise<{ rev: number; categories: unknown; printers: unknown }> {
    const path = `/api/desk/catalog?rev=${rev}`;
    const response = await this.fetchImpl(path, { credentials: "include", signal });
    if (signal?.aborted) throw new DOMException("aborted", "AbortError");
    if (response.status === 401) throw unauthorized(path);
    if (!response.ok) throw new Error(`${path} ${response.status}`);
    const data = unwrapData<{ rev: unknown; categories: unknown; printers: unknown }>(await response.json());
    if (!data || typeof data !== "object" || typeof data.rev !== "number") throw new Error("catalog");
    return { rev: data.rev, categories: data.categories, printers: data.printers };
  }

  private async getJson<T>(path: string): Promise<T> {
    const response = await this.fetchImpl(path, { credentials: "include" });
    if (response.status === 401) throw unauthorized(path);
    if (!response.ok) {
      const route = path.split("?")[0] ?? path;
      throw new Error(`${route} ${response.status}`);
    }
    return unwrapData<T>(await response.json());
  }

  private async send(path: string, body: BodyInit, headers?: HeadersInit): Promise<unknown> {
    const response = await this.fetchImpl(path, { method: "POST", credentials: "include", body, headers });
    if (response.status === 401) throw unauthorized(path);
    if (!response.ok) throw new Error(`${path} ${response.status}`);
    return response.json();
  }
}
