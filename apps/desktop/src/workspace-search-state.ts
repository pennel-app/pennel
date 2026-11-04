import type { WorkspaceTask } from "./workspace-state";

/** One search control keeps an independent query for each primary task. */
export class TaskSearchQueries {
  private readonly queries: Record<WorkspaceTask, string> = { dictation: "", meetings: "" };
  private task: WorkspaceTask;
  constructor(task: WorkspaceTask) { this.task = task; }
  set(value: string): void { this.queries[this.task] = value; }
  select(task: WorkspaceTask): string { this.task = task; return this.queries[task]; }
}
