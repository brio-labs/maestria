import { forwardRef, useRef, type KeyboardEvent } from 'react';

type Props = {
  query: string;
  selectedId: string | null;
  disabled: boolean;
  onChange: (query: string) => void;
  onMove: (delta: -1 | 1) => void;
  onSubmit: () => void;
  onEscape: () => void;
};

export const SearchInput = forwardRef<HTMLInputElement, Props>(function SearchInput(
  { query, selectedId, disabled, onChange, onMove, onSubmit, onEscape },
  ref,
) {
  const composing = useRef(false);
  function handleKeyDown(event: KeyboardEvent<HTMLInputElement>) {
    if (composing.current || event.nativeEvent.isComposing) return;
    if (event.key === 'ArrowDown') {
      event.preventDefault();
      onMove(1);
    } else if (event.key === 'ArrowUp') {
      event.preventDefault();
      onMove(-1);
    } else if (event.key === 'Enter') {
      event.preventDefault();
      onSubmit();
    } else if (event.key === 'Escape') {
      event.preventDefault();
      onEscape();
    }
  }

  return (
    <input
      ref={ref}
      className="search-input"
      type="search"
      value={query}
      placeholder="Search apps and commands…"
      aria-activedescendant={!disabled && selectedId ? `result-${selectedId}` : undefined}
      aria-label="Search apps and commands"
      role="combobox"
      aria-controls={!disabled && selectedId ? 'launcher-results' : undefined}
      aria-expanded={!disabled && selectedId !== null}
      aria-autocomplete="list"
      disabled={disabled}
      onChange={(event) => onChange(event.currentTarget.value)}
      onCompositionStart={() => { composing.current = true; }}
      onCompositionEnd={() => { composing.current = false; }}
      onKeyDown={handleKeyDown}
    />
  );
});
