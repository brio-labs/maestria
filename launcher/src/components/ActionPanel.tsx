import { useEffect, useRef, type KeyboardEvent } from 'react';
import type { Action } from '../model';

type Props = {
  actions: Action[];
  selectedIndex: number;
  disabled: boolean;
  onMove: (delta: -1 | 1) => void;
  onSelect: (index: number) => void;
  onExecute: (action: Action) => void;
  onClose: () => void;
};

export function ActionPanel({ actions, selectedIndex, disabled, onMove, onSelect, onExecute, onClose }: Props) {
  const buttons = useRef<Array<HTMLButtonElement | null>>([]);

  useEffect(() => {
    buttons.current[selectedIndex]?.focus();
  }, [selectedIndex]);

  function handleKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    if (event.nativeEvent.isComposing) return;
    if (event.key === 'ArrowDown') {
      event.preventDefault();
      onMove(1);
    } else if (event.key === 'ArrowUp') {
      event.preventDefault();
      onMove(-1);
    } else if (event.key === 'Enter') {
      event.preventDefault();
      const action = actions[selectedIndex];
      if (action) onExecute(action);
    } else if (event.key === 'Escape') {
      event.preventDefault();
      onClose();
    }
  }

  return (
    <div className="action-panel" role="menu" aria-label="Actions" tabIndex={-1} onKeyDown={handleKeyDown}>
      {actions.map((action, index) => (
        <button
          key={action.id}
          ref={(element) => { buttons.current[index] = element; }}
          type="button"
          role="menuitem"
          className={index === selectedIndex ? 'is-selected' : undefined}
          disabled={disabled}
          onFocus={() => onSelect(index)}
          onClick={() => onExecute(action)}
        >
          <span>{action.title}</span>
          {action.primary ? <span className="action-primary">Primary</span> : null}
        </button>
      ))}
    </div>
  );
}
