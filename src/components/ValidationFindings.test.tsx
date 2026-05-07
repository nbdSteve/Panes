import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import ValidationFindings from "./ValidationFindings";

describe("ValidationFindings", () => {
  it("renders nothing when findings empty", () => {
    const { container } = render(<ValidationFindings findings={[]} />);
    expect(container.firstChild).toBeNull();
  });

  it("renders severity, message, span, and source hint for each finding", () => {
    render(
      <ValidationFindings
        findings={[
          {
            severity: "error",
            message: "path missing",
            span: "src/gone.rs",
            source_hint: "workspace",
          },
          {
            severity: "warning",
            message: "path escapes workspace",
            span: "/etc/passwd",
            source_hint: null,
          },
          {
            severity: "info",
            message: "fyi only",
            span: null,
            source_hint: null,
          },
        ]}
      />,
    );
    expect(screen.getByText("path missing")).toBeInTheDocument();
    expect(screen.getByText("src/gone.rs")).toBeInTheDocument();
    expect(screen.getByText("source: workspace")).toBeInTheDocument();
    expect(screen.getByText("path escapes workspace")).toBeInTheDocument();
    expect(screen.getByText("fyi only")).toBeInTheDocument();
    expect(screen.getByText("error")).toBeInTheDocument();
    expect(screen.getByText("warning")).toBeInTheDocument();
    expect(screen.getByText("info")).toBeInTheDocument();
  });
});
