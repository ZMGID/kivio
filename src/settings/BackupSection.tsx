import { ErrorNotice } from '../components/AppNotice'
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
  const disabled = controller.busy
  return <div className="backup-page">
    <section className="backup-section"><h3 className="backup-section-title"><HardDrive size={16} />{zh ? '本地备份' : 'Local backup'}</h3><div className="backup-surface">
      <div className="backup-row">
        <div className="backup-copy"><p className="kv-row-label">{zh ? '配置备份' : 'Configuration backup'}</p><p className="kv-row-desc">{zh ? '设置、供应商、模型、助手和集。' : 'Settings, providers, models, assistants and sets.'}</p></div>
        <Button size="sm" disabled={disabled} onClick={controller.exportBackup}><Upload size={13} />{zh ? '导出配置' : 'Export configuration'}</Button>
      </div>
      <div className="backup-divider">
        <div className="backup-row">
          <div className="backup-copy"><p className="kv-row-label">{zh ? '从文件恢复' : 'Restore from a file'}</p><p className="kv-row-desc">{zh ? '选择 JSON 配置备份，恢复设置、供应商、模型、助手和集。' : 'Choose a JSON configuration backup to restore settings, providers, models, assistants and sets.'}</p></div>
          <Button size="sm" disabled={disabled} onClick={trusted ? controller.restoreBackup : controller.importBackup}><Download size={13} />{zh ? '导入备份' : 'Import backup'}</Button>
        </div>
        <div className="backup-trust">
          <Toggle ariaLabel={zh ? '信任此备份' : 'Trust this backup'} checked={trusted} onChange={setTrusted} disabled={disabled} />
          <span className="kv-row-desc">{zh ? '信任此备份，保留执行权限' : 'Trust this backup and retain execution permissions'}</span>
        </div>
      </div>
    </div></section>
    <WebDavBackupSection controller={controller} lang={lang} />
    <section className="backup-section"><h3 className="backup-section-title"><Layers size={16} />{zh ? '备份范围' : 'Backup scope'}</h3><div className="backup-surface">
      <div className="backup-scope-row"><span className="backup-tag">{zh ? '配置' : 'Configuration'}</span><p className="kv-row-desc">{zh ? '设置 · 供应商 · 模型 · 助手 · 集' : 'Settings · Providers · Models · Assistants · Sets'}</p></div>
    </div></section>
    <div className="backup-note"><ShieldCheck size={16} /><span>{zh ? '备份含密钥，请妥善保管。备份不包含对话、附件或项目文件。' : 'Backups contain secrets. Keep them safe. Conversations, attachments and project files are excluded.'}</span></div>
    <ErrorNotice message={controller.statusSource === 'local' && !controller.busy && controller.status?.kind === 'err' ? controller.status.msg : null} />
    {controller.statusSource === 'local' && (controller.busy || (controller.status && controller.status.kind !== 'err')) && <p role="status" aria-live="polite" className="text-sm whitespace-pre-wrap [overflow-wrap:anywhere]" style={{ color: controller.status?.kind === 'err' ? 'var(--danger)' : 'var(--text-muted)' }}>
      {controller.busy ? (zh ? '正在处理备份，请稍候…' : 'Processing backup…') : controller.status?.msg}
    </p>}
  </div>
}
