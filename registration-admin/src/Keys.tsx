import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { deleteKey, fetchKeys, patchKey, postKey, type KeyRow } from "./api";

type KeysProps = {
  /** The open event. Keys are reloaded when it changes. */
  eventId: string;
  /** Skip the outer card; the parent already wraps this block. */
  embedded?: boolean;
};

/**
 * Admin and operator keys of the open event. The event file starts with one of each;
 * the server refuses to delete the last key of either kind (`last_key`).
 */
export default function Keys({ eventId, embedded = false }: KeysProps) {
  const { t } = useTranslation();
  const [rows, setRows] = useState<KeyRow[] | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [editing, setEditing] = useState<number | null>(null);

  useEffect(() => {
    let cancel = false;
    setRows(null);
    setEditing(null);
    setError("");
    void fetchKeys()
      .then((result) => {
        if (cancel) {
          return;
        }
        if (!result.ok) {
          setError(result.code);
          setRows([]);
          return;
        }
        setRows(result.data);
      })
      .catch(() => {
        if (!cancel) {
          setError("unknown_error");
          setRows([]);
        }
      });
    return () => {
      cancel = true;
    };
  }, [eventId]);

  async function run(action: () => Promise<void>) {
    if (busy) {
      return;
    }
    setBusy(true);
    setError("");
    try {
      await action();
    } catch {
      setError("unknown_error");
    } finally {
      setBusy(false);
    }
  }

  function add(isAdmin: boolean) {
    void run(async () => {
      const result = await postKey(isAdmin);
      if (!result.ok) {
        setError(result.code);
        return;
      }
      // Keep the kind the operator asked for, even if the reply omits or mis-types is_admin.
      const row: KeyRow = { ...result.data, isAdmin };
      setRows((now) => [...(now ?? []), row]);
      setEditing(row.id);
    });
  }

  function save(row: KeyRow, key: string, comment: string) {
    const text = key.trim();
    if (!text) {
      setError("bad_request");
      return;
    }
    void run(async () => {
      const result = await patchKey(row.id, text, comment.trim());
      if (!result.ok) {
        setError(result.code);
        return;
      }
      const note = comment.trim();
      setRows((now) =>
        (now ?? []).map((item) =>
          item.id === row.id ? { ...item, key: text, comment: note === "" ? null : note } : item,
        ),
      );
      setEditing(null);
    });
  }

  function remove(row: KeyRow) {
    void run(async () => {
      const result = await deleteKey(row.id);
      if (!result.ok) {
        setError(result.code);
        return;
      }
      setRows((now) => (now ?? []).filter((item) => item.id !== row.id));
      if (editing === row.id) {
        setEditing(null);
      }
    });
  }

  const admins = (rows ?? []).filter((row) => row.isAdmin);
  const operators = (rows ?? []).filter((row) => !row.isAdmin);

  const body =
    rows === null ? (
      <p className="text-xs text-zinc-500">{t("keys.loading")}</p>
    ) : (
      <div className="flex flex-col gap-3">
        <KeyList
          id="keys-admin"
          title={t("keys.admin")}
          isAdmin
          rows={admins}
          busy={busy}
          editing={editing}
          onAdd={add}
          onEdit={setEditing}
          onSave={save}
          onDelete={remove}
        />
        <KeyList
          id="keys-operator"
          title={t("keys.operator")}
          isAdmin={false}
          rows={operators}
          busy={busy}
          editing={editing}
          onAdd={add}
          onEdit={setEditing}
          onSave={save}
          onDelete={remove}
        />
      </div>
    );

  const alert = error ? (
    <p role="alert" className="mt-2 text-xs text-red-600">
      {t(`error.${error}`)}
    </p>
  ) : null;

  if (embedded) {
    return (
      <div className="border-t border-zinc-100 pt-3">
        {body}
        {alert}
      </div>
    );
  }

  return (
    <section
      aria-labelledby="keys-title"
      className="rounded-2xl border border-zinc-200 bg-white p-5 shadow-sm"
    >
      <h2 id="keys-title" className="text-sm font-medium text-zinc-500">
        {t("keys.title")}
      </h2>
      <div className="mt-3">{body}</div>
      {alert}
    </section>
  );
}

type KeyListProps = {
  id: string;
  title: string;
  isAdmin: boolean;
  rows: KeyRow[];
  busy: boolean;
  editing: number | null;
  onAdd: (isAdmin: boolean) => void;
  onEdit: (id: number | null) => void;
  onSave: (row: KeyRow, key: string, comment: string) => void;
  onDelete: (row: KeyRow) => void;
};

function KeyList({
  id,
  title,
  isAdmin,
  rows,
  busy,
  editing,
  onAdd,
  onEdit,
  onSave,
  onDelete,
}: KeyListProps) {
  const { t } = useTranslation();
  const last = rows.length <= 1;
  return (
    <section aria-labelledby={id}>
      <div className="flex items-center justify-between gap-2">
        <h3 id={id} className="text-xs font-medium text-zinc-700">
          {title}
        </h3>
        <button
          type="button"
          disabled={busy}
          data-kind={isAdmin ? "admin" : "operator"}
          aria-label={`${t("keys.add")}: ${title}`}
          onClick={(event) => {
            event.preventDefault();
            event.stopPropagation();
            onAdd(isAdmin);
          }}
          className="rounded-md border border-zinc-300 bg-white px-2 py-0.5 text-xs hover:bg-zinc-100 disabled:cursor-wait disabled:opacity-60"
        >
          {t("keys.add")}
        </button>
      </div>
      {rows.length === 0 ? (
        <p className="mt-1 text-xs text-zinc-500">{t("keys.empty")}</p>
      ) : (
        <ul className="mt-1.5 flex flex-wrap items-center gap-1.5">
          {rows.map((row) =>
            editing === row.id ? (
              <KeyEditor
                key={row.id}
                row={row}
                busy={busy}
                onSave={onSave}
                onCancel={() => onEdit(null)}
              />
            ) : (
              <li
                key={row.id}
                className="inline-flex max-w-full items-center gap-1.5 rounded-lg border border-zinc-200 bg-zinc-50 px-2 py-1"
              >
                <div className="min-w-0">
                  <p className="select-all truncate font-mono text-xs">{row.key}</p>
                  {row.comment ? (
                    <p className="truncate text-[11px] text-zinc-500">{row.comment}</p>
                  ) : null}
                </div>
                <button
                  type="button"
                  disabled={busy}
                  onClick={(event) => {
                    event.stopPropagation();
                    onEdit(row.id);
                  }}
                  className="shrink-0 rounded px-1.5 py-0.5 text-xs text-zinc-700 hover:bg-zinc-100 disabled:opacity-60"
                >
                  {t("keys.edit")}
                </button>
                <button
                  type="button"
                  disabled={busy || last}
                  title={last ? t("keys.last") : undefined}
                  onClick={(event) => {
                    event.stopPropagation();
                    onDelete(row);
                  }}
                  className="shrink-0 rounded px-1.5 py-0.5 text-xs text-red-600 hover:bg-red-50 disabled:cursor-not-allowed disabled:text-zinc-400 disabled:hover:bg-transparent"
                >
                  {t("keys.delete")}
                </button>
              </li>
            ),
          )}
        </ul>
      )}
    </section>
  );
}

type KeyEditorProps = {
  row: KeyRow;
  busy: boolean;
  onSave: (row: KeyRow, key: string, comment: string) => void;
  onCancel: () => void;
};

function KeyEditor({ row, busy, onSave, onCancel }: KeyEditorProps) {
  const { t } = useTranslation();
  const [key, setKey] = useState(row.key);
  const [comment, setComment] = useState(row.comment ?? "");
  return (
    <li className="w-full basis-full rounded-lg border border-zinc-400 bg-white px-2 py-1.5">
      <form
        className="flex flex-col gap-1.5 sm:flex-row sm:flex-wrap sm:items-end"
        onSubmit={(event) => {
          event.preventDefault();
          event.stopPropagation();
          onSave(row, key, comment);
        }}
      >
        <label className="flex min-w-[10rem] flex-1 flex-col gap-0.5 text-[11px] text-zinc-500">
          {t("keys.key")}
          <input
            value={key}
            onChange={(event) => setKey(event.target.value)}
            autoFocus
            className="rounded-md border border-zinc-300 px-2 py-1 font-mono text-xs text-zinc-900"
          />
        </label>
        <label className="flex min-w-[10rem] flex-1 flex-col gap-0.5 text-[11px] text-zinc-500">
          {t("keys.comment")}
          <input
            value={comment}
            onChange={(event) => setComment(event.target.value)}
            className="rounded-md border border-zinc-300 px-2 py-1 text-xs text-zinc-900"
          />
        </label>
        <div className="flex gap-1.5">
          <button
            type="button"
            onClick={(event) => {
              event.stopPropagation();
              onCancel();
            }}
            className="rounded-md px-2 py-0.5 text-xs text-zinc-700 hover:bg-zinc-100"
          >
            {t("keys.cancel")}
          </button>
          <button
            type="submit"
            disabled={busy || key.trim() === ""}
            className="rounded-md bg-zinc-900 px-2 py-0.5 text-xs text-white disabled:opacity-60"
          >
            {t("keys.save")}
          </button>
        </div>
      </form>
    </li>
  );
}
