// A Jira ticket's edit screen, in the panel beside the board.
//
// What is on it is Jira's answer, not ours: the fields the ticket's edit
// screen has, each as the control its type wants. A field of a type this does
// not edit is shown as its value, so the form is still the whole ticket; a
// field Jira will not let this account change is not on it at all.
//
// Only what changed is saved. The form is a snapshot, and writing back every
// field would quietly undo whatever somebody else changed in Jira since it
// was loaded.
//
// Rich text is Markdown here and Jira's document format there, and the trip
// is lossy for what Markdown cannot say -- tables, panels, mentions, images.
// The field says which of those it holds before anybody presses save, rather
// than after they wonder where the table went.

import { useEffect, useRef, useState } from "react";

import { api, messageOf } from "./api";
import { Waiting } from "./Empty";
import type { Editable } from "./gen/Editable";
import type { EditField } from "./gen/EditField";
import type { EditForm } from "./gen/EditForm";
import type { Markdown } from "./gen/Markdown";
import type { Task } from "./gen/Task";
import type { UserChoice } from "./gen/UserChoice";
import { Busy, Close, Find } from "./icons";
import { Select } from "./Select";

export function TicketEditor({
  task,
  onSaved,
  onCancel,
}: {
  task: Task;
  onSaved: () => void;
  onCancel: () => void;
}) {
  const [form, setForm] = useState<EditForm | null>(null);
  const [edits, setEdits] = useState<Record<string, Editable>>({});
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    let live = true;
    api.editForm(task.tracker, task.key).then(
      (f) => live && setForm(f),
      (e) => live && setError(messageOf(e)),
    );
    return () => {
      live = false;
    };
  }, [task.tracker, task.key]);

  const current = (f: EditField) => edits[f.id] ?? f.edit;
  const changed = (form?.fields ?? []).filter(
    (f) => edits[f.id] && JSON.stringify(edits[f.id]) !== JSON.stringify(f.edit),
  );

  const save = async () => {
    setSaving(true);
    setError(null);
    try {
      await api.saveTicket(
        task.tracker,
        task.key,
        changed.map((f) => ({ id: f.id, edit: edits[f.id]! })),
      );
      onSaved();
    } catch (e) {
      setError(messageOf(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <section
      className="ticket-edit"
      onKeyDown={(e) => {
        // Escape gives up an edit that changed nothing. One that did is not
        // thrown away by a key that also closes menus: cancel is a button.
        if (e.key === "Escape" && !e.defaultPrevented) {
          e.preventDefault();
          if (changed.length === 0) onCancel();
        }
        if (e.key === "Enter" && (e.metaKey || e.ctrlKey) && changed.length > 0 && !saving) {
          e.preventDefault();
          void save();
        }
      }}
    >
      {error && <p className="error">{error}</p>}
      {!form && !error && <Waiting />}
      {form &&
        form.fields.map((f) => (
          <div key={f.id} className={`edit-field ${f.edit.kind}`}>
            <label className="edit-label" htmlFor={`field-${f.id}`}>
              {f.name}
              {f.required && <span className="required"> *</span>}
            </label>
            <Control
              id={`field-${f.id}`}
              task={task}
              field={f}
              value={current(f)}
              onChange={(v) => setEdits((all) => ({ ...all, [f.id]: v }))}
            />
          </div>
        ))}
      <footer className="ticket-edit-foot">
        <span className="hint">
          {changed.length === 0
            ? "nothing changed"
            : `${changed.length} ${changed.length === 1 ? "field" : "fields"} changed · ctrl+enter saves`}
        </span>
        <button className="quiet" disabled={saving} onClick={onCancel}>
          cancel
        </button>
        <button className="go" disabled={saving || changed.length === 0} onClick={() => void save()}>
          {saving ? <Busy className="turning" /> : null} save
        </button>
      </footer>
    </section>
  );
}

/// The control one field's kind wants.
function Control({
  id,
  task,
  field,
  value,
  onChange,
}: {
  id: string;
  task: Task;
  field: EditField;
  value: Editable;
  onChange: (value: Editable) => void;
}) {
  switch (value.kind) {
    case "text":
      return (
        <input
          id={id}
          className="prose"
          value={value.value}
          onChange={(e) => onChange({ ...value, value: e.target.value })}
        />
      );
    case "doc":
      return (
        <MarkdownBox
          id={id}
          value={value.value}
          rows={field.id === "description" ? 10 : 4}
          onChange={(text) => onChange({ ...value, value: { ...value.value, text } })}
        />
      );
    case "number":
      return (
        <input
          id={id}
          type="number"
          step="any"
          value={value.value ?? ""}
          onChange={(e) =>
            onChange({
              ...value,
              value: e.target.value === "" ? null : Number(e.target.value),
            })
          }
        />
      );
    case "date":
      return (
        <input
          id={id}
          type="date"
          value={value.value ?? ""}
          onChange={(e) => onChange({ ...value, value: e.target.value || null })}
        />
      );
    case "select":
      return (
        <Select
          aria-label={field.name}
          value={value.value ?? ""}
          onChange={(v) => onChange({ ...value, value: v || null })}
          options={[
            ...(!field.required || value.value === null ? [{ value: "", label: "none" }] : []),
            ...value.options.map((o) => ({ value: o.id, label: o.name })),
          ]}
        />
      );
    case "multi":
      return (
        <div className="edit-chips" role="group" aria-label={field.name}>
          {value.options.map((o) => {
            const on = value.value.includes(o.id);
            return (
              <button
                key={o.id}
                type="button"
                className={`chip${on ? " on" : ""}`}
                aria-pressed={on}
                onClick={() =>
                  onChange({
                    ...value,
                    value: on ? value.value.filter((v) => v !== o.id) : [...value.value, o.id],
                  })
                }
              >
                {o.name}
              </button>
            );
          })}
        </div>
      );
    case "labels":
      return <LabelsBox id={id} value={value.value} onChange={(v) => onChange({ ...value, value: v })} />;
    case "user":
      return (
        <UserBox
          id={id}
          task={task}
          value={value.value}
          assignable={value.assignable}
          required={field.required}
          onChange={(v) => onChange({ ...value, value: v })}
        />
      );
    case "read-only":
      return (
        <span className="edit-readonly" title="change this in Jira">
          {value.value || "none"}
        </span>
      );
  }
}

/// Rich text as Markdown, with what saving will flatten said beside it.
export function MarkdownBox({
  id,
  value,
  rows,
  placeholder,
  autoFocus,
  onChange,
}: {
  id?: string;
  value: Markdown;
  rows: number;
  placeholder?: string;
  autoFocus?: boolean;
  onChange: (text: string) => void;
}) {
  return (
    <div className="markdown-box">
      <textarea
        id={id}
        rows={rows}
        value={value.text}
        placeholder={placeholder ?? "Markdown: **bold**, *italic*, `code`, - lists, [links](https://…)"}
        autoFocus={autoFocus}
        spellCheck
        onChange={(e) => onChange(e.target.value)}
      />
      {value.lost.length > 0 && (
        <p className="warn markdown-lost">
          This holds what Markdown cannot: {value.lost.join(", ")}. Saving it from here flattens
          those. Edit it in Jira to keep them.
        </p>
      )}
    </div>
  );
}

/// Labels as words: Jira's labels have no spaces, so a space is the
/// separator, and what was typed is kept as typed until it is left.
function LabelsBox({
  id,
  value,
  onChange,
}: {
  id: string;
  value: string[];
  onChange: (labels: string[]) => void;
}) {
  const [text, setText] = useState(value.join(" "));
  return (
    <input
      id={id}
      value={text}
      placeholder="space-separated"
      onChange={(e) => {
        setText(e.target.value);
        onChange(e.target.value.split(/[\s,]+/).filter(Boolean));
      }}
    />
  );
}

/// A person: who it is now, and a search for somebody else.
function UserBox({
  id,
  task,
  value,
  assignable,
  required,
  onChange,
}: {
  id: string;
  task: Task;
  value: UserChoice | null;
  assignable: boolean;
  required: boolean;
  onChange: (user: UserChoice | null) => void;
}) {
  const [query, setQuery] = useState("");
  const [found, setFound] = useState<UserChoice[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const asked = useRef(0);

  // A beat after the typing stops, and only the latest answer counts.
  useEffect(() => {
    if (query.trim().length < 2) {
      setFound(null);
      return;
    }
    const n = ++asked.current;
    const timer = setTimeout(() => {
      api.users(task.tracker, task.key, query, assignable).then(
        (v) => n === asked.current && (setFound(v), setError(null)),
        (e) => n === asked.current && setError(messageOf(e)),
      );
    }, 250);
    return () => clearTimeout(timer);
  }, [query, task.tracker, task.key, assignable]);

  return (
    <div className="user-box">
      <div className="user-current">
        <span>{value ? value.name : <span className="nobody">nobody</span>}</span>
        {value && !required && (
          <button
            type="button"
            className="quiet-icon"
            title={assignable ? "unassign" : "clear"}
            onClick={() => onChange(null)}
          >
            <Close aria-label="clear" />
          </button>
        )}
      </div>
      <label className="user-search">
        <Find aria-hidden />
        <input
          id={id}
          className="prose"
          value={query}
          placeholder={assignable ? "assign to…" : "find somebody…"}
          onChange={(e) => setQuery(e.target.value)}
        />
      </label>
      {error && <p className="error">{error}</p>}
      {found && (
        <ul className="user-found">
          {found.length === 0 && <li className="hint">nobody by that name</li>}
          {found.map((u) => (
            <li key={u.account_id}>
              <button
                type="button"
                className="quiet"
                onClick={() => {
                  onChange(u);
                  setQuery("");
                }}
              >
                {u.name}
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
