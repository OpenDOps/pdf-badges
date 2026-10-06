import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router";

import { FormClient } from "../form/client";
import { DeskHeader, ScreenFrame } from "../shell/header";

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
    <>
      <DeskHeader />
      <ScreenFrame screen="key" title={t("screen.key")}>
        <form
          onSubmit={(event) => {
            event.preventDefault();
            void submit();
          }}
        >
          <input
            placeholder={t("key.placeholder")}
            value={key}
            onChange={(event) => setKey(event.target.value)}
          />
          <button type="submit">{t("key.enter")}</button>
          {error ? <p>{error}</p> : null}
        </form>
      </ScreenFrame>
    </>
  );
}
