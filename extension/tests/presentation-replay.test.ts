import { describe, expect, it, beforeEach, vi } from "vitest";

import { createPresentationManager } from "../utils/presentation";
import type { AskSession } from "../utils/daemon/ws";

const FIXTURE = `
<div class="file" data-tagsearch-path="src/a.ts"><table>
  <tr><td class="blob-num" data-line-number="1"></td><td class="blob-code">a</td></tr>
  <tr><td class="blob-num" data-line-number="2"></td><td class="blob-code">b</td></tr>
</table></div>`;

/** Minimal AskSession stand-in: lets us push presentation_call frames. */
function fakeSession() {
  const handlers: Array<(f: unknown) => void> = [];
  const results: unknown[] = [];
  const session = {
    on: (_type: string, h: (f: unknown) => void) => {
      handlers.push(h);
      return () => {};
    },
    sendPresentationResult: (...args: unknown[]) => results.push(args),
  } as unknown as AskSession;
  return { session, emit: (f: unknown) => handlers.forEach((h) => h(f)), results };
}

const flush = () => new Promise((r) => setTimeout(r, 20));

describe("presentation replay", () => {
  beforeEach(() => {
    document.body.innerHTML = FIXTURE;
  });

  it("records successful calls as steps and replays them from a clean page", async () => {
    const m = createPresentationManager();
    const { session, emit } = fakeSession();
    m.attach(session);
    emit({ type: "presentation_call", call_id: "c1", tool: "highlight_lines",
      input: { file: "src/a.ts", start_line: 1, end_line: 1, color: "red" } });
    emit({ type: "presentation_call", call_id: "c2", tool: "highlight_lines",
      input: { file: "src/a.ts", start_line: 2, end_line: 2, color: "blue" } });
    emit({ type: "presentation_call", call_id: "c3", tool: "highlight_lines",
      input: { file: "nope.ts", start_line: 9, end_line: 9 } }); // fails: not recorded
    await flush();
    expect(m.steps.map((s) => s.input.start_line)).toEqual([1, 2]);
    expect(document.querySelectorAll('[data-libre-cr-tag="highlight"]').length).toBe(2);

    await m.replayTo(0); // only the first step is on the page
    const rows = document.querySelectorAll('[data-libre-cr-tag="highlight"]');
    expect(rows.length).toBe(1);
    expect(rows[0]!.getAttribute("data-libre-cr-color")).toBe("red");

    await m.replayTo(); // everything again
    expect(document.querySelectorAll('[data-libre-cr-tag="highlight"]').length).toBe(2);

    m.resetSteps();
    expect(m.steps).toEqual([]);
  });

  it("renders a caption chip for the label and honours the visibility switch", async () => {
    const m = createPresentationManager();
    const { session, emit } = fakeSession();
    m.attach(session);
    emit({ type: "presentation_call", call_id: "c1", tool: "highlight_lines",
      input: { file: "src/a.ts", start_line: 1, end_line: 2, color: "green", label: "Guard rail" } });
    await flush();
    const chips = document.querySelectorAll(".libre-cr-label");
    expect(chips.length).toBe(1); // first row of the range only
    expect(chips[0]!.textContent).toBe("Guard rail");
    expect(chips[0]!.closest("tr")!.querySelector("td.blob-num")!.getAttribute("data-line-number")).toBe("1");

    m.setLabelsVisible(false);
    expect(document.documentElement.classList.contains("libre-cr-hide-labels")).toBe(true);
    m.setLabelsVisible(true);
    expect(document.documentElement.classList.contains("libre-cr-hide-labels")).toBe(false);

    m.clearAll();
    expect(document.querySelectorAll(".libre-cr-label").length).toBe(0);
  });

  it("showStep shows exactly one step", async () => {
    const m = createPresentationManager();
    const { session, emit } = fakeSession();
    m.attach(session);
    emit({ type: "presentation_call", call_id: "c1", tool: "highlight_lines",
      input: { file: "src/a.ts", start_line: 1, end_line: 1, label: "one", detail: "first thing" } });
    emit({ type: "presentation_call", call_id: "c2", tool: "highlight_lines",
      input: { file: "src/a.ts", start_line: 2, end_line: 2, label: "two" } });
    await flush();
    await m.showStep(1);
    const rows = document.querySelectorAll('[data-libre-cr-tag="highlight"]');
    expect(rows.length).toBe(1);
    expect(rows[0]!.querySelector("td.blob-num")!.getAttribute("data-line-number")).toBe("2");
    expect(m.steps[0]!.input.detail).toBe("first thing");
  });
});

describe("label visibility teardown", () => {
  beforeEach(() => {
    document.body.innerHTML = FIXTURE;
    document.documentElement.classList.remove("libre-cr-hide-labels");
  });

  it("detachAll removes the global hide-labels class", () => {
    const m = createPresentationManager();
    m.setLabelsVisible(false);
    expect(document.documentElement.classList.contains("libre-cr-hide-labels")).toBe(true);
    m.detachAll();
    expect(document.documentElement.classList.contains("libre-cr-hide-labels")).toBe(false);
  });

  it("a fresh manager clears a class a dead one left behind", () => {
    document.documentElement.classList.add("libre-cr-hide-labels");
    createPresentationManager();
    expect(document.documentElement.classList.contains("libre-cr-hide-labels")).toBe(false);
  });
});

describe("scroll_to scrolling is reviewer-initiated", () => {
  beforeEach(() => {
    document.body.innerHTML = FIXTURE;
  });

  it("does not scroll on live dispatch, does scroll on showStep", async () => {
    const row = document.querySelector('td[data-line-number="1"]')!.closest("tr")! as HTMLElement;
    const spy = vi.fn();
    (row as unknown as { scrollIntoView: () => void }).scrollIntoView = spy;
    const m = createPresentationManager();
    const { session, emit } = fakeSession();
    m.attach(session);
    emit({ tool: "scroll_to", call_id: "c1", input: { file: "src/a.ts", line: 1 } });
    await new Promise((r) => setTimeout(r, 20));
    expect(m.steps.length).toBe(1); // recorded…
    expect(spy).not.toHaveBeenCalled(); // …but the viewport did not move
    await m.showStep(0); // reviewer-initiated
    expect(spy).toHaveBeenCalled();
  });
});
