import { useState } from 'react'
import { Download, Upload } from 'lucide-react'
import { Button } from '../components/Button'
import { Toggle } from './public/controls'
import { SettingsGroup } from './components'
import type { Lang } from '../components/i18n'
import type { useSettingsBackupController } from './useSettingsBackupController'

export function BackupSection({ controller, lang }: {
  controller: ReturnType<typeof useSettingsBackupController>
  lang: Lang
}) {
  const [trusted, setTrusted] = useState(false)
  const zh = lang === 'zh'
  const disabled = controller.busy || controller.restartRequired
  return <SettingsGroup title={zh ? '备份与恢复' : 'Backup & restore'}>
    <div className="flex flex-wrap items-start gap-x-6 gap-y-2 pb-2">
      <div className="min-w-0">
        <Button size="sm" disabled={disabled} onClick={controller.exportBackup}><Upload size={11} />{zh ? '配置导出' : 'Export configuration'}</Button>
        <p className="kv-row-desc mt-2">{zh ? '设置、供应商、模型、助手和集。' : 'Settings, providers, models, assistants and sets.'}</p>
      </div>
      <div className="min-w-0">
        <Button size="sm" disabled={disabled} onClick={controller.exportFullBackup}><Upload size={11} />{zh ? '完全导出' : 'Export everything'}</Button>
        <p className="kv-row-desc mt-2">{zh ? '全部 Kivio 数据、对话、附件及项目文件。' : 'All Kivio data, conversations, attachments and project files.'}</p>
      </div>
    </div>
    <div className="flex flex-wrap items-center gap-3 border-t py-2" style={{ borderColor: 'var(--border)' }}>
      <div className="flex flex-wrap items-center gap-2">
        <Button size="sm" disabled={disabled} onClick={trusted ? controller.restoreBackup : controller.importBackup}><Download size={11} />{zh ? '导入备份' : 'Import backup'}</Button>
        {controller.restartRequired && <Button size="sm" disabled={controller.busy} onClick={controller.restart}>{zh ? '重启并完成恢复' : 'Restart and restore'}</Button>}
      </div>
      <div className="flex items-center gap-2">
        <Toggle ariaLabel={zh ? '信任此备份' : 'Trust this backup'} checked={trusted} onChange={setTrusted} disabled={disabled} />
        <span className="kv-row-desc">{zh ? '信任备份，保留权限与任务状态' : 'Trust backup and retain permissions and task states'}</span>
      </div>
    </div>
    <div className="flex flex-wrap items-baseline gap-x-4 gap-y-2">
      <p className="kv-row-desc">{zh ? '完全恢复将替换当前数据，重启生效。' : 'Full restore replaces current data after restart.'}</p>
    <details className="text-sm" style={{ color: 'var(--text-muted)' }}>
      <summary className="cursor-pointer">{zh ? '备份须知' : 'Backup details'}</summary>
      <p>{zh
        ? '备份含密钥，请勿分享。完全导出前请停止任务，保存到项目目录之外。项目文件恢复到独立目录；外部 CLI 历史、系统授权及外部登录需另行迁移。不信任备份时，恢复会停用插件、MCP、hooks 和任务，并保留本机 CLI 配置。'
        : 'Backups contain secrets. Stop tasks before full export and save outside project folders. Projects restore into separate folders. External CLI history, OS permissions and external sign-ins require separate migration. Untrusted restore disables plugins, MCP, hooks and tasks, and retains local CLI configuration.'}</p>
    </details>
    </div>
    {(controller.busy || controller.status) && <p role="status" aria-live="polite" className="text-sm" style={{ color: controller.status?.kind === 'err' ? 'var(--danger)' : 'var(--text-muted)' }}>
      {controller.busy ? (zh ? '正在处理备份，请稍候…' : 'Processing backup…') : controller.status?.msg}
    </p>}
  </SettingsGroup>
}
