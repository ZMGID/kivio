import { useState } from 'react'
import './backup.css'
import { WebDavBackupSection } from './WebDavBackupSection'
import { Download, Upload, HardDrive, Layers, ShieldCheck } from 'lucide-react'
import { Button } from '../components/Button'
import { Toggle } from './public/controls'
import type { Lang } from '../components/i18n'
import type { useSettingsBackupController } from './useSettingsBackupController'

export function BackupSection({ controller, lang }: {
  controller: ReturnType<typeof useSettingsBackupController>
  lang: Lang
}) {
  const [trusted, setTrusted] = useState(false)
  const zh = lang === 'zh'
  const disabled = controller.busy || controller.restartRequired
  return <div className="backup-page">
    {controller.restartRequired && <div className="backup-row backup-restart">
      <div className="backup-copy"><p className="kv-row-label">{zh ? '备份已就绪，重启后完成恢复' : 'Ready to restore after restart'}</p><p className="kv-row-desc">{zh ? '重启前，当前数据仍保持不变。' : 'Current data stays unchanged until restart.'}</p></div>
      <Button size="sm" variant="primary" disabled={controller.busy} onClick={controller.restart}>{zh ? '重启并完成恢复' : 'Restart and restore'}</Button>
    </div>}
    <section className="backup-section"><h3 className="backup-section-title"><HardDrive size={16} />{zh ? '本地备份' : 'Local backup'}</h3><div className="backup-surface">
      <div className="backup-row">
        <div className="backup-copy"><p className="kv-row-label">{zh ? '配置备份' : 'Configuration backup'}</p><p className="kv-row-desc">{zh ? '设置、供应商、模型、助手和集。' : 'Settings, providers, models, assistants and sets.'}</p></div>
        <Button size="sm" disabled={disabled} onClick={controller.exportBackup}><Upload size={13} />{zh ? '导出配置' : 'Export configuration'}</Button>
      </div>
      <div className="backup-row">
        <div className="backup-copy"><p className="kv-row-label">{zh ? '完全备份' : 'Full backup'}</p><p className="kv-row-desc">{zh ? '全部 Kivio 数据，包含对话、附件和项目文件。' : 'All Kivio data, including conversations, attachments and project files.'}</p></div>
        <Button size="sm" disabled={disabled} onClick={controller.exportFullBackup}><Upload size={13} />{zh ? '导出全部' : 'Export everything'}</Button>
      </div>
      <div className="backup-divider">
        <div className="backup-row">
          <div className="backup-copy"><p className="kv-row-label">{zh ? '从文件恢复' : 'Restore from a file'}</p><p className="kv-row-desc">{zh ? '选择配置或完全备份。完全恢复会替换当前数据，重启生效。' : 'Choose a configuration or full backup. Full restore replaces current data after restart.'}</p></div>
          <Button size="sm" disabled={disabled} onClick={trusted ? controller.restoreBackup : controller.importBackup}><Download size={13} />{zh ? '导入备份' : 'Import backup'}</Button>
        </div>
        <div className="backup-trust">
          <Toggle ariaLabel={zh ? '信任此备份' : 'Trust this backup'} checked={trusted} onChange={setTrusted} disabled={disabled} />
          <span className="kv-row-desc">{zh ? '信任此备份，保留权限与任务状态' : 'Trust this backup and retain permissions and task states'}</span>
        </div>
      </div>
    </div></section>
    <WebDavBackupSection controller={controller} lang={lang} />
    <section className="backup-section"><h3 className="backup-section-title"><Layers size={16} />{zh ? '备份范围' : 'Backup scope'}</h3><div className="backup-surface">
      <div className="backup-scope-row"><span className="backup-tag">{zh ? '配置' : 'Configuration'}</span><p className="kv-row-desc">{zh ? '设置 · 供应商 · 模型 · 助手 · 集' : 'Settings · Providers · Models · Assistants · Sets'}</p></div>
      <div className="backup-scope-row"><span className="backup-tag">{zh ? '完全' : 'Full'}</span><p className="kv-row-desc">{zh ? '全部配置，以及对话、附件和项目文件' : 'All configuration, conversations, attachments and project files'}</p></div>
    </div></section>
    <div className="backup-note"><ShieldCheck size={16} /><span>{zh ? '备份含密钥，请妥善保管。完全导出前请停止任务。' : 'Backups contain secrets. Keep them safe and stop tasks before full export.'}</span></div>
    {controller.statusSource === 'local' && (controller.busy || controller.status) && <p role="status" aria-live="polite" className="text-sm" style={{ color: controller.status?.kind === 'err' ? 'var(--danger)' : 'var(--text-muted)' }}>
      {controller.busy ? (zh ? '正在处理备份，请稍候…' : 'Processing backup…') : controller.status?.msg}
    </p>}
  </div>
}
