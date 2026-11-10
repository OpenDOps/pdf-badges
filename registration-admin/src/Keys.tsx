import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { deleteKey, fetchKeys, patchKey, postKey, type KeyRow } from "./api";

type KeysProps = {
  /** The open event. Keys are reloaded when it changes. */
  eventId: string;
};

/**
 * Admin and operator keys of the open event. The event file starts with one of each;
 * the server refuses to delete the last key of either kind (`last_key`).
 */
export default function Keys({ eventId }: KeysProps) {
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
      setRows((now) => [...(now ?? []), result.data]);
      setEditing(result.data.id);
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

  return (
    <section
      aria-labelledby="keys-title"
      className="rounded-2xl border border-zinc-200 bg-white p-5 shadow-sm"
    >
      <h2 id="keys-title" className="text-sm font-medium text-zinc-500">
        {t("keys.title")}
      </h2>
      {rows === null ? (
        <p className="mt-2 text-sm text-zinc-500">{t("keys.loading")}</p>
      ) : (
        <div className="mt-3 flex flex-col gap-5">
          <KeyList
            id="keys-admin"
            title={t("keys.admin")}
            rows={admins}
            busy={busy}
            editing={editing}
            onAdd={() => add(true)}
            onEdit={setEditing}
            onSave={save}
            onDelete={remove}
          />
          <KeyList
            id="keys-operator"
            title={t("keys.operator")}
            rows={operators}
            busy={busy}
            editing={editing}
            onAdd={() => add(false)}
            onEdit={setEditing}
            onSave={save}
            onDelete={remove}
          />
        </div>
      )}
      {error ? (
        <p role="alert" className="mt-3 text-sm text-red-600">
          {t(`error.${error}`)}
        </p>
      ) : null}
    </section>
  );
}

type KeyListProps = {
  id: string;
  title: string;
  rows: KeyRow[];
  busy: boolean;
  editing: number | null;
  onAdd: () => void;
  onEdit: (id: number | null) => void;
  onSave: (row: KeyRow, key: string, comment: string) => void;
  onDelete: (row: KeyRow) => void;
};

function KeyList({ id, title, rows, busy, editing, onAdd, onEdit, onSave, onDelete }: KeyListProps) {
  const { t } = useTranslation();
  const last = rows.length <= 1;
  return (
    <section aria-labelledby={id}>
      <div className="flex items-center justify-between gap-3">
        <h3 id={id} className="text-sm font-medium text-zinc-800">
          {title}
        </h3>
        <button
          type="button"
          disabled={busy}
          onClick={onAdd}
          className="rounded-lg border border-zinc-300 bg-white px-3 py-1 text-sm hover:bg-zinc-100 disabled:cursor-wait disabled:opacity-60"
        >
          {t("keys.add")}
        </button>
      </div>
      {rows.length === 0 ? (
        <p className="mt-2 text-sm text-zinc-500">{t("keys.empty")}</p>
      ) : (
        <ul className="mt-2 flex flex-col gap-2">
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
                className="flex items-center gap-3 rounded-xl border border-zinc-200 px-3 py-2"
              >
                <div className="min-w-0 flex-1">
                  <p className="select-all truncate font-mono text-sm">{row.key}</p>
                  {row.comment ? (
                    <p className="truncate text-xs text-zinc-500">{row.comment}</p>
                  ) : null}
                </div>
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => onEdit(row.id)}
                  className="rounded-lg px-2 py-1 text-sm text-zinc-700 hover:bg-zinc-100 disabled:opacity-60"
                >
                  {t("keys.edit")}
                </button>
                <button
                  type="button"
                  disabled={busy || last}
                  title={last ? t("keys.last") : undefined}
                  onClick={() => onDelete(row)}
                  className="rounded-lg px-2 py-1 text-sm text-red-600 hover:bg-red-50 disabled:cursor-not-allowed disabled:text-zinc-400 disabled:hover:bg-transparent"
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
    <li className="rounded-xl border border-zinc-400 px-3 py-2">
      <form
        className="flex flex-col gap-2"
        onSubmit={(event) => {
          event.preventDefault();
          onSave(row, key, comment);
        }}
      >
        <label className="flex flex-col gap-1 text-xs text-zinc-500">
          {t("keys.key")}
          <input
            value={key}
            onChange={(event) => setKey(event.target.value)}
            autoFocus
            className="rounded-lg border border-zinc-300 px-2 py-1 font-mono text-sm text-zinc-900"
          />
        </label>
        <label className="flex flex-col gap-1 text-xs text-zinc-500">
          {t("keys.comment")}
          <input
            value={comment}
            onChange={(event) => setComment(event.target.value)}
            className="rounded-lg border border-zinc-300 px-2 py-1 text-sm text-zinc-900"
          />
        </label>
        <div className="flex justify-end gap-2">
          <button
            type="button"
            onClick={onCancel}
            className="rounded-lg px-3 py-1 text-sm text-zinc-700 hover:bg-zinc-100"
          >
            {t("keys.cancel")}
          </button>
          <button
            type="submit"
            disabled={busy || key.trim() === ""}
            className="rounded-lg bg-zinc-900 px-3 py-1 text-sm text-white disabled:opacity-60"
          >
            {t("keys.save")}
          </button>
        </div>
      </form>
    </li>
  );
}
