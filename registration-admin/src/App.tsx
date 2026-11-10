import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import {
  fetchBinding,
  fetchEvents,
  fetchUpdate,
  postLogin,
  postSelect,
  type Event,
} from "./api";
import Events from "./Events";
import Login from "./Login";

export default function App() {
  const [path, setPath] = useState(window.location.pathname);

  useEffect(() => {
    const onPop = () => setPath(window.location.pathname);
    window.addEventListener("popstate", onPop);
    return () => window.removeEventListener("popstate", onPop);
  }, []);

  const openEvents = useCallback(() => {
    window.history.pushState({}, "", "/admin/events");
    setPath("/admin/events");
  }, []);

  const openLogin = useCallback(() => {
    window.history.pushState({}, "", "/admin");
    setPath("/admin");
  }, []);

  if (path === "/admin/events") {
    return <EventsRoute onLeave={openLogin} />;
  }

  return <LoginRoute onSuccess={openEvents} />;
}

function LoginRoute({ onSuccess }: { onSuccess: () => void }) {
  const [error, setError] = useState("");

  return (
    <Login
      error={error}
      onSubmit={async (login, password) => {
        const result = await postLogin(login, password);
        if (!result.ok) {
          setError(result.code);
          return;
        }
        setError("");
        onSuccess();
      }}
    />
  );
}

const UPDATE_POLL_MS = 60_000;

function EventsRoute({ onLeave }: { onLeave: () => void }) {
  const { t } = useTranslation();
  const [ready, setReady] = useState(false);
  const [events, setEvents] = useState<Event[]>([]);
  const [current, setCurrent] = useState<Event | null>(null);
  const [tokenStored, setTokenStored] = useState(false);
  const [version, setVersion] = useState<string | null>(null);
  const [newer, setNewer] = useState<string | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  const [checkFailed, setCheckFailed] = useState(false);
  const [retryIn, setRetryIn] = useState<number | null>(null);
  const [supervised, setSupervised] = useState(true);
  const [error, setError] = useState("");

  useEffect(() => {
    let cancel = false;
    async function load() {
      try {
        const list = await fetchEvents();
        if (cancel) {
          return;
        }
        if (!list.ok) {
          if (list.code === "session_expired") {
            onLeave();
            return;
          }
          setError(list.code);
          setReady(true);
          return;
        }
        const binding = await fetchBinding();
        if (cancel) {
          return;
        }
        setEvents(list.events);
        setCurrent(list.current);
        setTokenStored(binding.tokenStored);
        const update = await fetchUpdate();
        if (!cancel && update) {
          setVersion(update.version);
          setNewer(update.newer);
          setFailed(update.failed);
          setCheckFailed(update.checkFailed);
          setRetryIn(update.retryIn);
          setSupervised(update.supervised);
        }
        setReady(true);
      } catch {
        if (!cancel) {
          setError("unknown_error");
          setReady(true);
        }
      }
    }
    void load();
    return () => {
      cancel = true;
    };
  }, [onLeave]);

  // Pick up the server's own release checks (and a failed one) while the page stays open.
  useEffect(() => {
    if (!ready) {
      return;
    }
    const timer = window.setInterval(() => {
      void fetchUpdate()
        .then((update) => {
          if (!update) {
            return;
          }
          setVersion(update.version);
          setNewer(update.newer);
          setCheckFailed(update.checkFailed);
          setRetryIn(update.retryIn);
        })
        .catch(() => {});
    }, UPDATE_POLL_MS);
    return () => window.clearInterval(timer);
  }, [ready]);

  if (!ready) {
    return null;
  }
  if (error) {
    return (
      <main className="min-h-screen bg-zinc-50 px-4 py-10 text-zinc-900">
        <p role="alert" className="text-sm text-red-600">
          {t(`error.${error}`)}
        </p>
      </main>
    );
  }

  return (
    <Events
      events={events}
      current={current}
      tokenStored={tokenStored}
      version={version}
      newer={newer}
      failed={failed}
      checkFailed={checkFailed}
      retryIn={retryIn}
      supervised={supervised}
      onSelect={async (id, name) => {
        const result = await postSelect(id, name);
        if (!result.ok) {
          if (result.code === "session_expired") {
            onLeave();
            return;
          }
          return result.code;
        }
        setTokenStored(result.tokenStored);
        setCurrent({ id: result.id, name: result.name });
      }}
    />
  );
}
