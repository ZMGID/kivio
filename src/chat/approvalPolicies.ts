/** Built-in agent tool-approval policies. Shared by InputBar + PermissionPicker (kept in its
 *  own module so component files only export components — react-refresh lint). */
export const APPROVAL_POLICY_OPTIONS = [
  {
    value: 'always_confirm',
    label: '每次确认',
    title: '请求批准',
    description: '所有工具调用都先问你',
  },
  {
    value: 'readonly_auto_sensitive_confirm',
    label: '敏感确认',
    title: '替我审批',
    description: '只对写文件、终端等风险操作确认',
  },
  {
    value: 'auto',
    label: '完全访问',
    title: '完全访问权限',
    description: '工具调用自动放行',
  },
]

export function approvalPolicyOption(policy?: string) {
  return (
    APPROVAL_POLICY_OPTIONS.find((option) => option.value === policy)
    ?? APPROVAL_POLICY_OPTIONS[1]
  )
}
