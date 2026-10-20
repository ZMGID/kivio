import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeAll, describe, expect, it, vi } from 'vitest'
import { WebDavBackupSection } from './WebDavBackupSection'
import { useSettingsBackupController, type SettingsBackupPort } from './useSettingsBackupController'

beforeAll(() => {
  HTMLDialogElement.prototype.showModal ??= function () { this.setAttribute('open', '') }
  HTMLDialogElement.prototype.close ??= function () { this.removeAttribute('open') }
})

// Protects user-entered credentials and destination selection across a failed save.
// The transport stub rejects persistence; the real controller and form own the behavior.
describe('WebDAV configuration form', () => {
  it('retains the password on save failure and prevents transfers to an unsaved destination', async () => {
    const user = userEvent.setup()
    const config = { url: 'https://old.example.com', username: 'user', directory: 'Kivio', hasPassword: true }
    const save = vi.fn().mockRejectedValueOnce(new Error('Disk full')).mockImplementationOnce(async (input) => ({ ...config, url: input.url }))
    const port: SettingsBackupPort = {
      pickImport: async () => null, pickExport: async () => null, import: async () => {}, export: async () => {},
      importFull: async () => {}, confirmFullRestore: async () => false, pendingRestore: async () => false, restart: async () => {},
      webdav: {
        load: async () => config, save, test: async () => {}, list: async () => [], upload: async () => [],
        download: async () => ({ id: '', path: '' }), release: async () => {}, delete: async () => {},
        confirmRestore: async () => false, confirmDelete: async () => false,
      },
    }
    function Fixture() {
      const controller = useSettingsBackupController(port, 'en')
      return <WebDavBackupSection controller={controller} lang="en" />
    }
    render(<Fixture />)
    await user.click(await screen.findByRole('button', { name: 'Manage connection' }))
    const password = await screen.findByLabelText('WebDAV password')
    const url = screen.getByLabelText('WebDAV server URL')
    expect(password).toHaveValue('')
    await user.clear(url)
    await user.type(url, 'https://new.example.com')
    await user.type(password, 'new-secret')
    expect(screen.getByRole('button', { name: 'Upload backup' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Restore backup' })).toBeDisabled()
    await user.click(screen.getByRole('button', { name: 'Save configuration' }))
    expect(await screen.findByRole('status')).toHaveTextContent('Disk full')
    expect(password).toHaveValue('new-secret')
    await user.click(screen.getByRole('button', { name: 'Save configuration' }))
    expect(await screen.findByRole('status')).toHaveTextContent('WebDAV configuration saved')
    expect(password).toHaveValue('')
    expect(screen.getByRole('button', { name: 'Upload backup' })).toBeEnabled()
    await user.click(screen.getByRole('button', { name: 'Close' }))
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    expect(save.mock.calls[1][0]).toMatchObject({ password: 'new-secret', url: 'https://new.example.com' })
    await user.click(screen.getByRole('button', { name: 'Manage connection' }))
    await user.click(screen.getByRole('button', { name: 'Choose service' }))
    await user.click(screen.getByRole('button', { name: 'Choose Jianguoyun' }))
    expect(screen.getByLabelText('WebDAV server URL')).toHaveValue('https://dav.jianguoyun.com/dav/')
    expect(screen.getByLabelText('WebDAV username')).toHaveValue('')
    expect(screen.getByLabelText('WebDAV password')).toHaveValue('')
    await user.click(screen.getByRole('button', { name: 'Choose service' }))
    await user.click(screen.getByRole('button', { name: 'Choose Nextcloud' }))
    expect(screen.getByLabelText('WebDAV server URL')).toHaveValue('')
    expect(screen.getByLabelText('WebDAV server URL')).toHaveAttribute('placeholder', 'https://cloud.example.com/remote.php/dav/files/USERNAME/')
    expect(screen.getByRole('button', { name: 'Upload backup' })).toBeDisabled()

  })
})
