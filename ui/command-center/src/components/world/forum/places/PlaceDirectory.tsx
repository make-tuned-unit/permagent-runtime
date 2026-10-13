/**
 * World W2 — the non-spatial equivalent: every place and its primary action
 * as a plain list, reachable without walking, pointer lock or camera travel.
 * W6 mounts it in the Places list; choosing an entry opens that place's panel.
 */
import { Button } from '../../../common/Button';
import { PLACE_SCHEMAS } from '../data/placeSchema';
import type { WorldPlaceId } from '../data/identity';
import { placeColors, placeStyles } from './placeStyles';
import { W2_PLACE_IDS } from './integration';

export function PlaceDirectory({ onOpenPlace, places = W2_PLACE_IDS }: { onOpenPlace(placeId: WorldPlaceId): void; places?: readonly WorldPlaceId[] }) {
  const colors = placeColors();
  const styles = placeStyles(colors, true);
  return (
    <nav aria-label="Places">
      <ul style={styles.list}>
        {PLACE_SCHEMAS.filter((p) => places.includes(p.id)).map((p) => (
          <li key={p.id} style={styles.row} data-place-entry={p.id}>
            <Button colors={colors} flashSuccess={false} aria-describedby={`world-place-desc-${p.id}`} onClick={() => onOpenPlace(p.id)}>{p.accessible.listLabel}</Button>
            <span id={`world-place-desc-${p.id}`} style={styles.note}>{p.accessible.description}</span>
          </li>
        ))}
      </ul>
    </nav>
  );
}
