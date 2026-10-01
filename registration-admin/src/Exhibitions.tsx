import { useTranslation } from "react-i18next";

export type Exhibition = {
  id: string;
  name: string;
};

type ExhibitionsProps = {
  exhibitions: Exhibition[];
  current: Exhibition | null;
  tokenStored: boolean;
  onSelect: (id: string, name: string) => void;
};

export default function Exhibitions({
  exhibitions,
  current,
  tokenStored,
  onSelect,
}: ExhibitionsProps) {
  const { t } = useTranslation();

  return (
    <main className="min-h-screen bg-zinc-50 px-4 py-10 text-zinc-900">
      <div className="mx-auto flex w-full max-w-lg flex-col gap-6">
        <h1 className="text-2xl font-semibold tracking-tight">
          {t("exhibitions.title")}
        </h1>
        <section
          aria-labelledby="current-exhibition"
          className="rounded-2xl border border-zinc-200 bg-white p-5 shadow-sm"
        >
          <h2
            id="current-exhibition"
            className="text-sm font-medium text-zinc-500"
          >
            {t("exhibitions.current")}
          </h2>
          <p className="mt-1 text-lg font-medium">
            {current?.name ?? ""}
          </p>
        </section>
        <section aria-labelledby="choose-exhibition">
          <h2 id="choose-exhibition" className="text-sm font-medium text-zinc-500">
            {t("exhibitions.choose")}
          </h2>
          {exhibitions.length === 0 ? (
            <p className="mt-3 rounded-2xl border border-dashed border-zinc-300 bg-white px-4 py-8 text-center text-sm text-zinc-500">
              {t("exhibitions.empty")}
            </p>
          ) : (
            <ul className="mt-3 flex flex-col gap-2">
              {exhibitions.map((exhibition) => {
                const selected = exhibition.id === current?.id;
                return (
                  <li key={exhibition.id}>
                    <button
                      type="button"
                      onClick={() => onSelect(exhibition.id, exhibition.name)}
                      className={
                        selected
                          ? "w-full rounded-xl border border-zinc-900 bg-zinc-900 px-4 py-3 text-left text-sm font-medium text-white"
                          : "w-full rounded-xl border border-zinc-200 bg-white px-4 py-3 text-left text-sm font-medium hover:border-zinc-400"
                      }
                    >
                      {exhibition.name}
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </section>
        <p
          role="status"
          className={
            tokenStored
              ? "rounded-xl bg-emerald-50 px-4 py-3 text-sm text-emerald-800"
              : "rounded-xl bg-zinc-100 px-4 py-3 text-sm text-zinc-600"
          }
        >
          {tokenStored
            ? t("exhibitions.token_stored")
            : t("exhibitions.token_empty")}
        </p>
        <section
          aria-labelledby="keys-title"
          className="rounded-2xl border border-dashed border-zinc-300 bg-white p-5"
        >
          <h2 id="keys-title" className="text-sm font-medium text-zinc-500">
            {t("keys.title")}
          </h2>
          <p className="mt-1 text-sm text-zinc-500">{t("keys.empty")}</p>
        </section>
      </div>
    </main>
  );
}
