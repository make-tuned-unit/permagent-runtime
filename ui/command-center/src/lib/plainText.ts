/** Flatten markdown to one line of plain text for previews that are not
 * rendered as markdown (list rows, clamped snippets): drops heading hashes,
 * bold/italic/code markers and link syntax, and collapses whitespace. */
export function plainText(markdown: string): string {
  return markdown
    .replace(/!?\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/^\s{0,3}#{1,6}\s+/gm, '')
    .replace(/(\*\*|__|`)/g, '')
    .replace(/\s+/g, ' ')
    .trim();
}
