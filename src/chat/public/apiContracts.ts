export type {
  Automation,
  AutomationChangedEvent,
  AutomationMeta,
  AutomationRun,
  AutomationRunEvent,
  AutomationRunStarted,
  AutomationRunSummary,
  NodeOutput,
  ValidationIssue,
} from '../automation/types'
export type { GoalState } from '../types'
export {
  normalizeGitDiffStat,
  normalizeGitRepoState,
  type GitSnapshot,
} from '../dock/types'
