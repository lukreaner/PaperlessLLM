import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { defaultSettings } from "../api/defaults";
import { ProcessingControls } from "./SettingsControls";

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
