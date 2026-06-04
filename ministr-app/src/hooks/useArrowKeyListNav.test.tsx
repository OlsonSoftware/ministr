import { render, fireEvent } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { useArrowKeyListNav } from "./useArrowKeyListNav";

function List() {
  const ref = useArrowKeyListNav<HTMLDivElement>();
  return (
    <div ref={ref} data-testid="list">
      {[0, 1, 2].map((i) => (
        <button key={i} type="button" data-roving-item data-testid={`row-${i}`}>
          row {i}
        </button>
      ))}
      <input data-testid="not-a-row" />
    </div>
  );
}

describe("useArrowKeyListNav", () => {
  it("moves focus down/up/Home/End among [data-roving-item] rows", () => {
    const { getByTestId } = render(<List />);
    const row0 = getByTestId("row-0");
    const row1 = getByTestId("row-1");
    const row2 = getByTestId("row-2");

    row0.focus();
    expect(document.activeElement).toBe(row0);

    fireEvent.keyDown(row0, { key: "ArrowDown" });
    expect(document.activeElement).toBe(row1);

    fireEvent.keyDown(row1, { key: "ArrowDown" });
    expect(document.activeElement).toBe(row2);

    // Clamp at the end.
    fireEvent.keyDown(row2, { key: "ArrowDown" });
    expect(document.activeElement).toBe(row2);

    fireEvent.keyDown(row2, { key: "ArrowUp" });
    expect(document.activeElement).toBe(row1);

    fireEvent.keyDown(row1, { key: "Home" });
    expect(document.activeElement).toBe(row0);

    fireEvent.keyDown(row0, { key: "End" });
    expect(document.activeElement).toBe(row2);
  });

  it("does not hijack arrows when focus is outside the rows (e.g. an input)", () => {
    const { getByTestId } = render(<List />);
    const input = getByTestId("not-a-row");
    input.focus();
    fireEvent.keyDown(input, { key: "ArrowDown" });
    // focus stays on the input — the hook ignores non-row focus
    expect(document.activeElement).toBe(input);
  });
});
