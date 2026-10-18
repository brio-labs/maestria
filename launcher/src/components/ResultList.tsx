import { Fragment, useEffect, useRef } from 'react';
import neutralAppAsset from '../assets/neutral-app.svg';
import type { FileSelection, Result } from '../model';
import { CalculatorResult } from './CalculatorResult';

type Props = {
  results: Result[];
  selectedId: string | null;
  fileSelection: FileSelection | null;
  actionInFlight: boolean;
  onSelect: (id: string) => void;
  onSubmit: () => void;
};

function safeIcon(icon: string | null): string {
  return icon?.startsWith('data:image/png;base64,') ? icon : neutralAppAsset;
}

function ResultRow({ result, selected, actionInFlight, onSelect, onSubmit }: {
  result: Result;
  selected: boolean;
  actionInFlight: boolean;
  onSelect: (id: string) => void;
  onSubmit: () => void;
}) {
  return (
    <button
      id={`result-${result.id}`}
      type="button"
      role="option"
      aria-selected={selected}
      className={`result-row${selected ? ' is-selected' : ''}`}
      disabled={actionInFlight}
      onClick={() => onSelect(result.id)}
      onDoubleClick={onSubmit}
      onMouseMove={(event) => { if (event.movementX || event.movementY) onSelect(result.id); }}
      title={[result.title, result.subtitle].filter(Boolean).join('\n')}
    >
      <img className="result-icon" src={safeIcon(result.icon)} alt="" aria-hidden="true" onError={(event) => {
        if (event.currentTarget.getAttribute('src') !== neutralAppAsset) event.currentTarget.src = neutralAppAsset;
      }} />
      <span className="result-copy">
        <span className="result-title">{result.title}</span>
        {result.subtitle ? <span className="result-subtitle">{result.subtitle}</span> : null}
      </span>
    </button>
  );
}

const labels: Record<Result['kind'], string> = {
  application: 'Applications', command: 'Commands', calculation: 'Calculation',
};

export function ResultList({ results, selectedId, fileSelection, actionInFlight, onSelect, onSubmit }: Props) {
  const list = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const selected = list.current?.querySelector('[aria-selected="true"]');
    selected?.scrollIntoView({ block: 'nearest' });
  }, [selectedId, results]);
  const visibleResults: Result[] = fileSelection ? [{
    id: fileSelection.resultId, kind: 'command', title: fileSelection.displayPath,
    subtitle: 'Selected file', icon: null, actions: [],
  }] : results;

  return (
    <div ref={list} id="launcher-results" className="result-list" role="listbox" aria-label="Launcher results">
      {visibleResults.map((result, index) => (
        <Fragment key={result.id}>
          {(index === 0 || visibleResults[index - 1]?.kind !== result.kind) ? (
            <div className="section-label" role="presentation">{fileSelection ? 'Selected file' : labels[result.kind]}</div>
          ) : null}
          {result.kind === 'calculation' ? (
            <CalculatorResult result={result} selected={selectedId === result.id} actionInFlight={actionInFlight}
              onSelect={onSelect} onSubmit={onSubmit} />
          ) : (
            <ResultRow result={result} selected={selectedId === result.id} actionInFlight={actionInFlight}
              onSelect={onSelect} onSubmit={onSubmit} />
          )}
        </Fragment>
      ))}
    </div>
  );
}
