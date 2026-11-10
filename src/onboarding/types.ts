export type OnboardingStepId =
  | 'welcome'
  | 'appearance'
  | 'provider'
  | 'webSearch'
  | 'hotkey'
  | 'done'

// 界面语言首次按系统 locale 自动选定（见 OnboardingShell 的 detectSystemLang），
// 在「外观」步可以当场改；之后仍可在「设置 → 基础」里调整。
export const ONBOARDING_STEPS: OnboardingStepId[] = [
  'welcome',
  'appearance',
  'provider',
  'webSearch',
  'hotkey',
  'done',
]
