import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ReviewTaxonomyEditor } from "./ReviewPage";

describe("ReviewTaxonomyEditor", () => {
  it("allows a new name while retaining catalog suggestions when unrestricted", () => {
    const onChange = vi.fn();
    render(<ReviewTaxonomyEditor label="Suggested correspondent" value="Acme GmbH" choices={["Acme GmbH", "Example Ltd"]} allowNew onChange={onChange} />);

    const input = screen.getByRole("combobox", { name: "Suggested correspondent" });
    const listId = input.getAttribute("list");
    expect(listId).toBeTruthy();
    expect(document.querySelector(`#${listId} option[value="Example Ltd"]`)).toBeInTheDocument();

    fireEvent.change(input, { target: { value: "A newly corrected sender" } });
    expect(onChange).toHaveBeenLastCalledWith("A newly corrected sender");
  });

  it("limits editing to catalog choices when restricted", () => {
    const onChange = vi.fn();
    render(<ReviewTaxonomyEditor label="Suggested document type" value="Invoice" choices={["Invoice", "Receipt"]} allowNew={false} onChange={onChange} />);

    const select = screen.getByRole("combobox", { name: "Suggested document type" });
    expect(select).not.toHaveAttribute("list");
    fireEvent.change(select, { target: { value: "Receipt" } });
    expect(onChange).toHaveBeenLastCalledWith("Receipt");
  });
});
