// Guided tour of the assistant's highlights: one step at a time, driven by
// the reviewer. Shows the step's title (the highlight label) and the model's
// explanation (`detail`) so the diff can be followed without the transcript.

import type { PresentationStep } from "../utils/presentation";

import { Markdown } from "./Markdown";

export interface TourWidgetProps {
  steps: PresentationStep[];
  index: number;
  onStep: (index: number) => void;
  onShowAll: () => void;
  onClose: () => void;
  /** Opened by the assistant's own presentation calls: show a single
   *  "Scroll to first highlight" action and scroll nothing until it is clicked. */
  armed?: boolean;
  onStart?: () => void;
}

function stepTitle(step: PresentationStep, i: number): string {
  const label = step.input.label;
  if (typeof label === "string" && label.trim()) return label;
  const file = typeof step.input.file === "string" ? step.input.file.split("/").pop() : "";
  return `${step.tool} ${file}`.trim() || `Step ${i + 1}`;
}

export function TourWidget({
  steps,
  index,
  onStep,
  onShowAll,
  onClose,
  armed = false,
  onStart,
}: TourWidgetProps) {
  if (armed) {
    return (
      <div className="libre-cr-tour" role="dialog" aria-label="Highlight tour" data-testid="tour">
        <div className="libre-cr-tour-nav">
          <button className="libre-cr-tour-btn primary" onClick={onStart} data-testid="tour-start">
            Scroll to first highlight
          </button>
          <span className="libre-cr-tour-count" data-testid="tour-count">
            {steps.length} highlight{steps.length === 1 ? "" : "s"}
          </span>
          <button className="libre-cr-tour-btn" onClick={onClose} aria-label="Close tour">
            ✕
          </button>
        </div>
      </div>
    );
  }
  const step = steps[index];
  if (!step) return null;
  const detail = typeof step.input.detail === "string" ? step.input.detail : "";
  const file = typeof step.input.file === "string" ? step.input.file : "";
  const line = step.input.start_line ?? step.input.line;
  const where = file ? `${file}${typeof line === "number" ? `:${line}` : ""}` : "";
  return (
    <div className="libre-cr-tour" role="dialog" aria-label="Highlight tour" data-testid="tour">
      <div className="libre-cr-tour-nav">
        <button
          className="libre-cr-tour-btn"
          disabled={index <= 0}
          onClick={() => onStep(index - 1)}
          aria-label="Previous step"
        >
          ◀ Prev
        </button>
        <span className="libre-cr-tour-count" data-testid="tour-count">
          {index + 1} / {steps.length}
        </span>
        <button
          className="libre-cr-tour-btn primary"
          disabled={index >= steps.length - 1}
          onClick={() => onStep(index + 1)}
          aria-label="Next step"
        >
          Next ▶
        </button>
        <button className="libre-cr-tour-btn" onClick={onShowAll} title="Lay every highlight down at once">
          Show all
        </button>
        <button className="libre-cr-tour-btn" onClick={onClose} aria-label="Close tour">
          ✕
        </button>
      </div>
      <div className="libre-cr-tour-body">
        <div className="libre-cr-tour-title">{stepTitle(step, index)}</div>
        {where ? <div className="libre-cr-tour-where">{where}</div> : null}
        {detail ? (
          <div className="libre-cr-tour-detail" data-testid="tour-detail">
            <Markdown text={detail} />
          </div>
        ) : null}
      </div>
    </div>
  );
}
