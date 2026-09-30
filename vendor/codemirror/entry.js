// Bundled by vendor/update-codemirror.sh into codemirror.min.js (global `CM`).
export { EditorView, keymap, lineNumbers, highlightActiveLine, highlightActiveLineGutter,
         drawSelection, dropCursor, rectangularSelection, crosshairCursor,
         highlightSpecialChars, placeholder } from "@codemirror/view";
export { EditorState, Compartment, EditorSelection } from "@codemirror/state";
export { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
export { syntaxHighlighting, defaultHighlightStyle, HighlightStyle, indentOnInput,
         bracketMatching, foldGutter, foldKeymap } from "@codemirror/language";
export { SearchQuery, setSearchQuery, getSearchQuery, findNext, findPrevious,
         search, searchKeymap, highlightSelectionMatches } from "@codemirror/search";
export { markdown, markdownLanguage } from "@codemirror/lang-markdown";
export { languages } from "@codemirror/language-data";
export { oneDark } from "@codemirror/theme-one-dark";
export { tags } from "@lezer/highlight";
