import { describe, expect, it, vi } from "vitest";
import type { JobView } from "./types";

const listJobs = vi.fn<(limit?: number) => Promise<JobView[]>>();

vi.mock("./api", () => ({
  api: { listJobs: (limit?: number) => listJobs(limit) },
  asDonnaError: (value: unknown) => value,
  hasBackend: () => false,
  subscribe: async () => () => {},
}));

const { getState, refreshJobs, setState } = await import("./store");

const job = (id: string, status: JobView["status"]): JobView => ({
  job_id: id,
  status,
  goal: "goal",
  summary: null,
  submitted_at: "2026-10-26T10:00:00Z",
  updated_at: "2026-10-26T10:00:00Z",
});

describe("refreshJobs", () => {
  it("runs one trailing refresh for calls made while one is in flight", async () => {
    setState({ core: "ok" });
    let release: (jobs: JobView[]) => void = () => {};
    listJobs.mockImplementationOnce(() => new Promise((resolve) => (release = resolve)));
    listJobs.mockResolvedValueOnce([job("j1", "completed")]);

    const first = refreshJobs();
    void refreshJobs();
    void refreshJobs();
    release([job("j1", "running")]);
    await first;

    expect(listJobs).toHaveBeenCalledTimes(2);
    expect(getState().jobs?.[0]?.status).toBe("completed");
  });

  it("does not ask a Core that is not connected", async () => {
    listJobs.mockClear();
    setState({ core: "unreachable" });
    await refreshJobs();
    expect(listJobs).not.toHaveBeenCalled();
  });
});
