/** Built-in agent tool-approval policies. Used by PermissionPicker (kept in its
 *  own module so component files only export components — react-refresh lint). */
export const APPROVAL_POLICY_OPTIONS = [
  {
    value: 'always_confirm',
    label: '每次确认',
    title: '请求批准',
    description: '文件和命令工具每次确认；免审批工具除外',
  },
  {
    value: 'readonly_auto_sensitive_confirm',
    label: '敏感确认',
    title: '替我审批',
    description: '文件和命令工具首次授权，MCP 调用逐次确认',
  },
  {
    value: 'auto',
    label: '完全访问',
    title: '完全访问权限',
    description: '已启用工具自动放行；不改变工具开关或系统文件权限',
  },
]
