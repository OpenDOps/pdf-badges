import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router";

import { FormClient } from "../form/client";
import { ScreenFrame } from "../shell/header";

function codeOf(error: unknown): string | undefined {
  if (!(error instanceof Error)) return undefined;
  const code = (error as { code?: unknown }).code;
  return typeof code === "string" ? code : undefined;
}

export default function Key() {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const client = useMemo(() => new FormClient(), []);
  const [key, setKey] = useState("");
  const [error, setError] = useState("");
  const [bound, setBound] = useState<{ id: string; name: string | null } | null | undefined>(undefined);

  useEffect(() => {
    let cancel = false;
    void client.boundEvent().then(
      (event) => {
        if (cancel) return;
        setBound(event.id ? { id: event.id, name: event.name } : null);
      },
      () => {
        if (!cancel) setBound(null);
      },
    );
    return () => {
      cancel = true;
    };
  }, [client]);

  async function submit() {
    if (!key.trim()) return;
    try {
      await client.auth(key);
      navigate("/");
    } catch (caught) {
      setError(codeOf(caught) === "invalid_key" ? t("error.invalid_key") : "");
    }
  }

  return (
    <ScreenFrame screen="key">
      <form
        onSubmit={(event) => {
          event.preventDefault();
          void submit();
        }}
      >
        {bound ? (
          <p data-event={bound.id} className="text-center text-sm text-slate-700">
            {bound.name ? <span className="block">{bound.name}</span> : null}
            <span className="block">{t("key.uid", { id: bound.id })}</span>
          </p>
        ) : null}
        {bound === null ? (
          <p className="text-center text-sm text-slate-700">
            <span className="block">{t("key.none")}</span>
            <a href="/admin" className="text-sky-700 underline">
              {t("key.select")}
            </a>
          </p>
        ) : null}
        <input
          placeholder={t("key.placeholder")}
          value={key}
          onChange={(event) => setKey(event.target.value)}
        />
        <button type="submit">{t("key.enter")}</button>
        {error ? <p>{error}</p> : null}
      </form>
    </ScreenFrame>
  );
}
