/** World W2 — says when a record could not be opened, instead of looking like it worked. */
import type { OpenResult } from '../data/recordLinks';
import type { PlaceStyles } from './placeStyles';

export function LinkResultNote({ linkResult, styles }: { linkResult: { key: string; result: OpenResult } | null; styles: PlaceStyles }) {
  if (!linkResult || linkResult.result.ok) return null;
  return <p style={styles.warn} role="alert" data-link-failed={linkResult.key}>{linkResult.result.reason}</p>;
}
