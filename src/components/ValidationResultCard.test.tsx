import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import ValidationResultCard from "./ValidationResultCard";

describe("ValidationResultCard", () => {
  it("renders pass without findings", () => {
    render(
      <ValidationResultCard
        validator="citation"
        outcome="pass"
        findings={[]}
        durationMs={5}
      />,
    );
    expect(screen.getByText("Passed")).toBeInTheDocument();
    expect(screen.getByText("citation")).toBeInTheDocument();
    expect(screen.getByText("5ms")).toBeInTheDocument();
  });

  it("renders fail with findings", () => {
    render(
      <ValidationResultCard
        validator="secret_scan"
        outcome="fail"
        findings={[
          { severity: "error", message: "AWS key", span: "AKIA…", source_hint: "built-in" },
        ]}
        durationMs={2}
      />,
    );
    expect(screen.getByText("Failed")).toBeInTheDocument();
    expect(screen.getByText("AWS key")).toBeInTheDocument();
    expect(screen.getByText("AKIA…")).toBeInTheDocument();
  });
});
