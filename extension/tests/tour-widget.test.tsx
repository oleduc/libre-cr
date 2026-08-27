import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";

import { TourWidget } from "../components/TourWidget";

const steps = [
  { tool: "highlight_lines", input: { file: "src/a.ts", start_line: 10, end_line: 12, label: "Guard rail", detail: "Public clients cannot hold client_credentials." } },
  { tool: "highlight_lines", input: { file: "src/b.ts", start_line: 3, end_line: 3, label: "Cache" } },
];

describe("TourWidget", () => {
  it("shows the step title, location and explanation, and navigates", () => {
    const onStep = vi.fn();
    render(<TourWidget steps={steps} index={0} onStep={onStep} onShowAll={() => {}} onClose={() => {}} />);
    expect(screen.getByText("Guard rail")).toBeTruthy();
    expect(screen.getByText("src/a.ts:10")).toBeTruthy();
    expect(screen.getByTestId("tour-detail").textContent).toContain("client_credentials");
    expect(screen.getByTestId("tour-count").textContent).toBe("1 / 2");
    expect((screen.getByLabelText("Previous step") as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByLabelText("Next step"));
    expect(onStep).toHaveBeenCalledWith(1);
  });
});
