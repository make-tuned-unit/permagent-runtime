/**
 * World W2 — a keyboard listbox of evidence records.
 *
 * Arrow keys / Home / End move between records, Enter or Space selects (the
 * native button click). Selecting is LOCAL: it shows the record's detail and
 * never sends, writes or approves anything. One tab stop for the whole list
 * (roving tabindex), so Tab moves on to the actions instead of through every row.
 */
import { forwardRef, useImperativeHandle, useRef, type KeyboardEvent, type ReactNode, type Ref } from 'react';
import type { PlaceStyles } from './placeStyles';

export interface RecordListboxHandle {
  /** Focus the selected record, or the first one. False when the list is empty. */
  focus(): boolean;
}

export interface RecordListboxProps<T> {
  label: string;
  items: readonly T[];
  keyOf(item: T): string;
  selectedKey: string | null;
  onSelect(key: string): void;
  renderOption(item: T, selected: boolean): ReactNode;
  styles: PlaceStyles;
}

function RecordListboxInner<T>({ label, items, keyOf, selectedKey, onSelect, renderOption, styles }: RecordListboxProps<T>, ref: Ref<RecordListboxHandle>) {
  const optionRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const selectedIndex = Math.max(0, items.findIndex((i) => keyOf(i) === selectedKey));

  useImperativeHandle(ref, () => ({
    focus() {
      const el = optionRefs.current[selectedIndex] ?? optionRefs.current[0];
      el?.focus({ preventScroll: false });
      return !!el;
    },
  }), [selectedIndex]);

  const move = (from: number, to: number) => {
    const clamped = Math.min(items.length - 1, Math.max(0, to));
    if (clamped === from) return;
    optionRefs.current[clamped]?.focus();
    onSelect(keyOf(items[clamped]));
  };

  const onKeyDown = (index: number) => (e: KeyboardEvent<HTMLButtonElement>) => {
    switch (e.key) {
      case 'ArrowDown': e.preventDefault(); move(index, index + 1); break;
      case 'ArrowUp': e.preventDefault(); move(index, index - 1); break;
      case 'Home': e.preventDefault(); move(index, 0); break;
      case 'End': e.preventDefault(); move(index, items.length - 1); break;
      default: break;
    }
  };

  return (
    <div role="listbox" aria-label={label} style={styles.list}>
      {items.map((item, index) => {
        const key = keyOf(item);
        const selected = key === selectedKey;
        return (
          <button
            key={key}
            ref={(el) => { optionRefs.current[index] = el; }}
            type="button"
            role="option"
            className="pa-btn pa-btn--composite"
            aria-selected={selected}
            tabIndex={index === selectedIndex ? 0 : -1}
            data-record-key={key}
            style={styles.option(selected)}
            onClick={() => onSelect(key)}
            onKeyDown={onKeyDown(index)}
          >
            {renderOption(item, selected)}
          </button>
        );
      })}
    </div>
  );
}

export const RecordListbox = forwardRef(RecordListboxInner) as <T>(props: RecordListboxProps<T> & { ref?: Ref<RecordListboxHandle> }) => ReturnType<typeof RecordListboxInner>;
