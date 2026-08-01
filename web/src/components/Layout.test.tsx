import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { Router } from "../lib/router";
import { Layout } from "./Layout";

describe("desktop service controls", () => {
  it("keeps an obvious quit action in the application sidebar", () => {
    const onQuit = vi.fn();
    render(
      <Router>
        <Layout onRefreshStatus={() => undefined} onLogout={() => undefined} onQuit={onQuit}>
          <p>Dashboard</p>
        </Layout>
      </Router>,
    );

    const quit = screen.getByRole("button", { name: "Quit PaperlessLLM" });
    expect(quit).toBeVisible();
    fireEvent.click(quit);
    expect(onQuit).toHaveBeenCalledOnce();
  });
});
