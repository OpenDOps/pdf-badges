import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { FormClient } from "../form/client";

export function versionLabel(version: string): string {
  return version.startsWith("v") ? version : `v${version}`;
}

export function ServerVersion({ admin }: { admin: boolean }) {
  const { t } = useTranslation();
  const [version, setVersion] = useState<string | null>(null);
  const [newer, setNewer] = useState<string | null>(null);

  useEffect(() => {
    let cancel = false;
    async function load() {
      try {
        const body = await new FormClient().update();
        if (cancel) return;
        setVersion(body.version);
        setNewer(body.newer);
      } catch {
        if (!cancel) {
          setVersion(null);
          setNewer(null);
        }
      }
    }
    void load();
    const timer = window.setInterval(() => void load(), 5000);
    return () => {
      cancel = true;
      window.clearInterval(timer);
    };
  }, []);

  if (!version && !newer) return null;
  return (
    <div data-version-stack>
      {version ? (
        <p data-version className="rounded-full border border-slate-200 bg-white px-2.5 py-1 text-sm text-slate-700 shadow-sm">
          {versionLabel(version)}
        </p>
      ) : null}
      {admin && newer ? (
        <a
          href="/admin"
          data-signal="warn"
          className="cursor-pointer rounded-full border border-amber-200 bg-amber-50 px-2.5 py-1 text-sm text-amber-900 shadow-sm hover:bg-amber-100"
        >
          {t("menu.updateAvailable", { version: versionLabel(newer) })}
        </a>
      ) : null}
    </div>
  );
}
