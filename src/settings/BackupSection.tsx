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
          <div className="backup-copy"><p className="kv-row-label">{zh ? '从文件恢复' : 'Restore from a file'}</p><p className="kv-row-desc">{zh ? '选择 ZIP 配置备份（兼容旧 JSON），恢复设置、供应商、模型、助手和集。' : 'Choose a ZIP configuration backup (legacy JSON supported) to restore settings, providers, models, assistants and sets.'}</p></div>
          <Button size="sm" disabled={disabled} onClick={trusted ? controller.restoreBackup : controller.importBackup}><Download size={13} />{zh ? '导入备份' : 'Import backup'}</Button>
        </div>
        <div className="backup-trust">
          <Toggle ariaLabel={zh ? '信任此备份' : 'Trust this backup'} checked={trusted} onChange={setTrusted} disabled={disabled} />
          <span className="kv-row-desc">{zh ? '信任此备份，保留执行权限' : 'Trust this backup and retain execution permissions'}</span>
        </div>
      </div>
    </div></section>
    <section className="backup-section">
      <h3 className="backup-section-title"><Layers size={16} />{zh ? '聊天全量备份' : 'Full chat backup'}</h3>
      <div className="backup-surface"><div className="backup-row">
        <div className="backup-copy"><p className="kv-row-label">{zh ? '项目、对话和附件' : 'Projects, conversations and attachments'}</p>
          <p className="kv-row-desc">{zh ? '包含助手、集、项目记录、全部对话、分支、草稿和 Kivio 保存的附件。恢复后重启，整体替换聊天数据。' : 'Includes assistants, sets, project records, all conversations, branches, drafts and attachments stored by Kivio. Restore replaces chat data and restarts the app.'}</p>
          <p className="kv-row-desc">{zh ? '不包含项目代码、外部链接文件或 CLI 原生会话。换电脑后可能需要重新指定项目目录，外部 CLI 不保证能续聊。' : 'Excludes project source files, externally linked files and CLI native sessions. Project paths may need updating on another computer; CLI session continuation is not guaranteed.'}</p>
        </div>
        <div className="backup-actions">
          <Button size="sm" disabled={disabled} onClick={controller.exportChatBackup}><Upload size={13} />{zh ? '导出全部聊天' : 'Export all chats'}</Button>
          <Button size="sm" disabled={disabled} onClick={controller.restoreChatBackup}><Download size={13} />{zh ? '恢复全部聊天' : 'Restore all chats'}</Button>
        </div>
      </div></div>
    </section>
    <WebDavBackupSection controller={controller} lang={lang} />
    <section className="backup-section"><h3 className="backup-section-title"><Layers size={16} />{zh ? '备份范围' : 'Backup scope'}</h3><div className="backup-surface">
      <div className="backup-scope-row"><span className="backup-tag">{zh ? '配置' : 'Configuration'}</span><p className="kv-row-desc">{zh ? '设置 · 供应商 · 模型 · 助手 · 集' : 'Settings · Providers · Models · Assistants · Sets'}</p></div>
    </div></section>
    <div className="backup-note"><ShieldCheck size={16} /><span>{zh ? '备份可能含密钥或私人聊天，请妥善保管。配置备份不含聊天；聊天全量备份不含项目代码。' : 'Backups may contain secrets or private chats. Keep them safe. Configuration backups exclude chats; chat backups exclude project source files.'}</span></div>
    <ErrorNotice message={controller.statusSource === 'local' && !controller.busy && controller.status?.kind === 'err' ? controller.status.msg : null} />
    {controller.statusSource === 'local' && (controller.busy || (controller.status && controller.status.kind !== 'err')) && <p role="status" aria-live="polite" className="text-sm whitespace-pre-wrap [overflow-wrap:anywhere]" style={{ color: controller.status?.kind === 'err' ? 'var(--danger)' : 'var(--text-muted)' }}>
      {controller.busy ? (zh ? '正在处理备份，请稍候…' : 'Processing backup…') : controller.status?.msg}
    </p>}
  </div>
}
