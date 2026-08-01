import { useState } from "react";
import { api } from "../api/client";
import { useApp } from "../App";
import { useToast } from "./Toast";
import { Button } from "./UI";

export function WorkerControls({ onChanged }: { onChanged?: () => void | Promise<void> }) {
  const { status, refreshStatus } = useApp();
  const { showToast } = useToast();
  const [changing, setChanging] = useState(false);
  const paused = status.worker.paused || status.worker.state === "paused";

  async function toggle() {
    setChanging(true);
    try {
      if (paused) {
        await api.resumeWorker();
        showToast("Processing resumed");
      } else {
        await api.pauseWorker();
        showToast(status.worker.active_jobs?.length || status.worker.active_job || status.worker.scan_progress ? "Processing will pause after the current operation" : "Processing paused");
      }
      await refreshStatus();
      await onChanged?.();
    } catch (caught) {
      showToast(caught instanceof Error ? caught.message : "Could not change worker state", "error");
    } finally {
      setChanging(false);
    }
  }

  return (
    <Button variant={paused ? "primary" : "secondary"} loading={changing} onClick={() => void toggle()}>
      <span aria-hidden="true">{paused ? "▶" : "Ⅱ"}</span> {paused ? "Continue processing" : "Pause processing"}
    </Button>
  );
}
