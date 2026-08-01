import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { defaultSettings } from "../api/defaults";
import { InferenceConcurrencyControl, ProcessingControls, WorkflowModeControl } from "./SettingsControls";

describe("ProcessingControls", () => {
  it("exposes every required processing control", () => {
    render(<ProcessingControls value={defaultSettings.processing} tags={[]} customFields={[]} onChange={() => undefined} />);
    expect(screen.getByText("Use existing Correspondents and Tags?")).toBeInTheDocument();
    expect(screen.getByText("Process only specific pre tagged documents?")).toBeInTheDocument();
    expect(screen.getByText("Restrict to Existing Tags")).toBeInTheDocument();
    expect(screen.getByText("Restrict to Existing Correspondents")).toBeInTheDocument();
    expect(screen.getByText("Restrict to Existing Document Types")).toBeInTheDocument();
    expect(screen.getByText("Add AI-processed tag to documents?")).toBeInTheDocument();
    expect(screen.getByText("Tags Assignment")).toBeInTheDocument();
    expect(screen.getByText("Correspondent Detection")).toBeInTheDocument();
    expect(screen.getByText("Document Type Classification")).toBeInTheDocument();
    expect(screen.getByText("Title Generation")).toBeInTheDocument();
    expect(screen.getByText("Custom Fields")).toBeInTheDocument();
    expect(screen.getByText("Custom Prompt")).toBeInTheDocument();
  });

  it("changes only the selected AI function", () => {
    const onChange = vi.fn();
    render(<ProcessingControls value={defaultSettings.processing} tags={[]} customFields={[]} onChange={onChange} />);
    const functionChecks = document.querySelectorAll<HTMLInputElement>(".function-card input");
    fireEvent.click(functionChecks[4]);
    expect(onChange).toHaveBeenCalledWith(expect.objectContaining({
      functions: { ...defaultSettings.processing.functions, custom_fields: true },
    }));
  });
});

describe("WorkflowModeControl", () => {
  it("makes automatic and review behavior explicit", () => {
    const onChange = vi.fn();
    render(<WorkflowModeControl reviewMode={false} onChange={onChange} />);

    expect(screen.getByRole("radio", { name: /Automatic/ })).toHaveAttribute("aria-checked", "true");
    expect(screen.getByText("Successful suggestions will be applied without approval.")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("radio", { name: /Review before applying/ }));
    expect(onChange).toHaveBeenCalledWith(true);
  });

  it("distinguishes a draft choice from the mode that is currently saved", () => {
    render(<WorkflowModeControl reviewMode savedReviewMode={false} onChange={() => undefined} />);

    expect(screen.getByRole("status")).toHaveTextContent("Review mode selected — save changes to activate it.");
    expect(screen.getByRole("status")).toHaveTextContent("Automatic mode remains active until you save.");
  });

  it("describes setup choices as pending until setup is finished", () => {
    render(<WorkflowModeControl reviewMode activationContext="setup" onChange={() => undefined} />);

    expect(screen.getByRole("status")).toHaveTextContent("Review mode selected.");
    expect(screen.getByRole("status")).toHaveTextContent("It will become active when you finish setup.");
    expect(screen.queryByText(/Review mode is active/)).not.toBeInTheDocument();
  });
});

describe("InferenceConcurrencyControl", () => {
  it("bounds parallel analyses between one and eight", () => {
    const onChange = vi.fn();
    const { rerender } = render(<InferenceConcurrencyControl value={1} onChange={onChange} />);
    const input = screen.getByLabelText(/Parallel AI analyses/);
    expect(input).toHaveAttribute("min", "1");
    expect(input).toHaveAttribute("max", "8");

    fireEvent.change(input, { target: { value: "12" } });
    expect(onChange).toHaveBeenLastCalledWith(8);
    rerender(<InferenceConcurrencyControl value={8} onChange={onChange} />);
    fireEvent.change(screen.getByLabelText(/Parallel AI analyses/), { target: { value: "0" } });
    expect(onChange).toHaveBeenLastCalledWith(1);
  });
});
