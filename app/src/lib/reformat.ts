// Reformatting SQL, in the style chosen in Settings.

import { format } from 'sql-formatter';

export type ReformatStyle = 'standard' | 'compact' | 'oneline' | 'aligned';
export type KeywordCase = 'upper' | 'lower' | 'preserve';

export const REFORMAT_STYLES: { value: ReformatStyle; label: string; note: string; sample: string }[] = [
  {
    value: 'standard',
    label: 'Standard',
    note: 'Each clause on its own line, with what belongs to it indented underneath.',
    sample: 'SELECT\n  id,\n  name\nFROM\n  people\nWHERE\n  born > 1990\n  AND city = \'Oslo\'',
  },
  {
    value: 'compact',
    label: 'Compact',
    note: 'One line for each clause: far fewer lines, still easy to scan.',
    sample: 'SELECT id, name\nFROM people\nWHERE born > 1990 AND city = \'Oslo\'',
  },
  {
    value: 'aligned',
    label: 'Aligned',
    note: 'Keywords in a column on the left, what follows them lined up on the right.',
    sample: 'SELECT    id,\n          name\nFROM      people\nWHERE     born > 1990\nAND       city = \'Oslo\'',
  },
  {
    value: 'oneline',
    label: 'One line',
    note: 'Each statement on a single line, for pasting into a script or a ticket.',
    sample: 'SELECT id, name FROM people WHERE born > 1990 AND city = \'Oslo\'',
  },
];

/** A line that ends in a comment running to the end of the line: nothing may be joined onto it. */
const endsInComment = (line: string) => /(^|\s)(--|#)/.test(line);

/** Joins two pieces of a statement, with a space unless a bracket or comma makes one wrong. */
function join(left: string, right: string): string {
  const piece = right.trim();
  return /[([]$/.test(left) || /^[)\],;]/.test(piece) ? left + piece : `${left} ${piece}`;
}

/**
 * Folds the standard layout into fewer lines. `whole` puts a statement on
 * one line; otherwise each clause (a line that starts at the margin) gets
 * one. A line comment keeps its line break, or it would swallow what follows.
 */
function fold(formatted: string, whole: boolean): string {
  return formatted
    .split(/\n{2,}/)
    .map((statement) => {
      const out: string[] = [];
      for (const line of statement.split('\n')) {
        const last = out.length - 1;
        const starts = !whole && !/^\s/.test(line);
        // A closing bracket at the margin belongs to the clause before it.
        if (last < 0 || endsInComment(out[last]) || (starts && !/^[)\]]/.test(line))) out.push(line.trimEnd());
        else out[last] = join(out[last], line);
      }
      return out.join('\n');
    })
    .join('\n\n');
}

/**
 * Reformats a piece of a script that is selected, so that it goes back
 * exactly where it was and nothing around it moves.
 *
 * The white space at each end of the selection is kept as it is: a selection
 * that takes in the line break after a statement must not pull the next
 * statement up onto its last line. `before` is what comes before the
 * selection on its first line. Every line after the first is given that
 * line's indent, so a piece that was indented stays indented as a block.
 *
 * Throws like `reformatSql` if the piece cannot be read as SQL, which a
 * piece cut through the middle of a bracket cannot.
 */
export function reformatPiece(selected: string, before: string, mariadb: boolean, style: ReformatStyle, keywords: KeywordCase, indent: number): string {
  const lead = /^\s*/.exec(selected)?.[0] ?? '';
  const trail = /\s*$/.exec(selected.slice(lead.length))?.[0] ?? '';
  const formatted = reformatSql(selected.slice(lead.length, selected.length - trail.length), mariadb, style, keywords, indent);
  // The line the piece starts on, up to where it starts: its indent is the margin for the rest.
  const startLine = (before + lead).split('\n').at(-1) ?? '';
  const margin = /^[ \t]*/.exec(startLine)?.[0] ?? '';
  const lines = formatted.split('\n').map((line, i) => (i === 0 || line === '' ? line : margin + line));
  return lead + lines.join('\n') + trail;
}

/** Reformats SQL. Throws, with a message from the parser, if the text cannot be read as SQL. */
export function reformatSql(source: string, mariadb: boolean, style: ReformatStyle, keywords: KeywordCase, indent: number): string {
  const formatted = format(source, {
    language: mariadb ? 'mariadb' : 'mysql',
    keywordCase: keywords,
    tabWidth: indent,
    indentStyle: style === 'aligned' ? 'tabularLeft' : 'standard',
    // {start} and {end}, as a query run in time slices marks its range, are left as they are.
    paramTypes: { custom: [{ regex: String.raw`\{\w+\}` }] },
  });
  return style === 'compact' ? fold(formatted, false) : style === 'oneline' ? fold(formatted, true) : formatted;
}
