// Both sides of one file, in Monaco, for the worktree's diff and a pull
// request's.
//
// A file that is new, or gone, has nothing on one side, and Monaco's diff of
// it is a column of nothing beside the file -- or, in one column, the file
// under an empty red line that "was" there. So such a file is shown on its own,
// in a plain editor, every line coloured as Monaco colours an added or removed
// one.

import * as monaco from "monaco-editor/editor/editor.api";

import type { FileDiff } from "../gen/FileDiff";
import { ensureTheme, languageOf, THEME } from "./File";

const COMMON = {
  theme: THEME,
  readOnly: true,
  domReadOnly: true,
  automaticLayout: true,
  minimap: { enabled: false },
  scrollBeyondLastLine: false,
  fontSize: 13,
} as const;

export type Shown = {
  /// The editor showing the file as it is now, to comment on; none when the
  /// file is gone.
  now: monaco.editor.ICodeEditor | null;
  dispose: () => void;
};

/// `diff`: the diff editor's own options, for a file that changed.
export function showSides(
  element: HTMLElement,
  sides: FileDiff,
  path: string,
  diff: monaco.editor.IDiffEditorConstructionOptions = {},
): Shown {
  ensureTheme();
  const language = languageOf(path);
  const added = sides.original === "" && sides.modified !== "";
  const removed = sides.modified === "" && sides.original !== "";

  if (added || removed) {
    const model = monaco.editor.createModel(added ? sides.modified : sides.original, language);
    const editor = monaco.editor.create(element, { ...COMMON, model });
    editor.createDecorationsCollection([
      {
        range: model.getFullModelRange(),
        options: { isWholeLine: true, className: added ? "line-insert" : "line-delete" },
      },
    ]);
    return {
      now: added ? editor : null,
      dispose: () => {
        model.dispose();
        editor.dispose();
      },
    };
  }

  const editor = monaco.editor.createDiffEditor(element, {
    ...COMMON,
    renderSideBySide: true,
    // The + and − beside the lines are drawn in an icon font, which on
    // Windows came out as boxes; the colours already say it.
    renderIndicators: false,
    ...diff,
  });
  const original = monaco.editor.createModel(sides.original, language);
  const modified = monaco.editor.createModel(sides.modified, language);
  editor.setModel({ original, modified });
  return {
    now: editor.getModifiedEditor(),
    dispose: () => {
      original.dispose();
      modified.dispose();
      editor.dispose();
    },
  };
}
