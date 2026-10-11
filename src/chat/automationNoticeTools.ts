export const READ_TOOLS = new Set(['automation_list', 'automation_get', 'automation_runs', 'schedule_list'])
const WRITE_TOOLS = new Set(['automation_upsert', 'automation_set_enabled', 'automation_delete', 'schedule_create', 'schedule_update', 'schedule_delete'])

export function isAutomationNoticeTool(name: string): boolean {
  return READ_TOOLS.has(name) || WRITE_TOOLS.has(name)
}
