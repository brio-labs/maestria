import type { Result } from '../model';

type Props = {
  result: Result;
  selected: boolean;
  actionInFlight: boolean;
  onSelect: (id: string) => void;
  onSubmit: () => void;
};

export function CalculatorResult({ result, selected, actionInFlight, onSelect, onSubmit }: Props) {
  return (
    <button
      id={`result-${result.id}`}
      type="button"
      role="option"
      aria-selected={selected}
      className={`result-row calculator-row${selected ? ' is-selected' : ''}`}
      disabled={actionInFlight}
      onClick={() => onSelect(result.id)}
      onDoubleClick={onSubmit}
      onMouseMove={(event) => { if (event.movementX || event.movementY) onSelect(result.id); }}
      title={[result.title, result.subtitle].filter(Boolean).join('\n')}
    >
      <span className="calculator-copy">
        <span className="calculator-expression">{result.subtitle ?? 'Calculation'}</span>
        <span className="calculator-value">{result.title}</span>
        <span className="calculator-expression">Approximate arithmetic</span>
      </span>
      <span className="result-kind">Copy Result</span>
    </button>
  );
}
