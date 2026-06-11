import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Keel } from "./Keel";

describe("Keel (seed)", () => {
  it("renders title and line", () => {
    render(<Keel title="ministr" line="rebuilding" />);
    expect(screen.getByRole("heading", { name: "ministr" })).toBeInTheDocument();
    expect(screen.getByText("rebuilding")).toBeInTheDocument();
  });
});
