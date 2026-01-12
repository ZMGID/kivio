import { PlatformAdapter } from './interface';
import { app, globalShortcut } from 'electron';
import { spawn } from 'child_process';
import path from 'path';
import fs from 'fs';

export class WindowsAdapter implements PlatformAdapter {
    platformName = 'win32' as const;

    // eslint-disable-next-line @typescript-eslint/no-unused-vars
    async captureScreenshot(_mode: 'selection' | 'full' = 'selection'): Promise<string | null> {
        return new Promise((resolve) => {
            const isDev = !app.isPackaged;
            const finalScriptPath = isDev
                ? path.join(app.getAppPath(), 'resources/scripts/win-screenshot.ps1')
                : path.join(process.resourcesPath, 'scripts', 'win-screenshot.ps1');

            const tempPath = path.join(app.getPath('temp'), `screenshot-${Date.now()}.png`);

            console.log('Using screenshot script:', finalScriptPath);

            const ps = spawn('powershell', [
                '-NoProfile',
                '-ExecutionPolicy', 'Bypass',
                '-File', finalScriptPath,
                tempPath
            ]);

            ps.on('close', (code: number) => {
                if (code === 0 && fs.existsSync(tempPath)) {
                    resolve(tempPath);
                } else {
                    console.error('Screenshot failed or cancelled. Exit code:', code);
                    resolve(null);
                }
            });

            ps.on('error', (err: Error) => {
                console.error('Failed to start powershell:', err);
                resolve(null);
            });
        });
    }

    async performSystemOCR(imagePath: string): Promise<string> {
        return new Promise((resolve, reject) => {
            const isDev = !app.isPackaged;
            const binaryName = 'keylingo-ocr.exe';

            // 统一使用 Release 路径
            const extraResourcePath = isDev
                ? path.join(__dirname, '../../native/win-ocr/bin/Release/net6.0-windows10.0.19041.0/keylingo-ocr.exe')
                : path.join(process.resourcesPath, 'ocr', binaryName);

            console.log('Using Windows OCR binary:', extraResourcePath);

            const ps = spawn(extraResourcePath, [imagePath]);

            let stdout = '';
            let stderr = '';

            ps.stdout.on('data', (data: Buffer) => { stdout += data.toString(); });
            ps.stderr.on('data', (data: Buffer) => { stderr += data.toString(); });

            ps.on('close', (code: number) => {
                if (code === 0) {
                    resolve(stdout.trim());
                } else {
                    console.error('Windows OCR Error:', stderr);
                    reject(new Error(stderr || 'Windows OCR failed'));
                }
            });

            ps.on('error', (err: Error) => {
                console.error('Failed to start Windows OCR:', err);
                reject(err);
            });
        });
    }

    getModifierKey(): string {
        return 'Ctrl';
    }

    registerHotkey(accelerator: string, callback: () => void): boolean {
        // 自动将 Command 映射为 Ctrl
        const winAccelerator = accelerator.replace('Command', 'Ctrl').replace('Cmd', 'Ctrl');
        return globalShortcut.register(winAccelerator, callback);
    }

    // eslint-disable-next-line @typescript-eslint/no-unused-vars
    async pasteText(_text: string): Promise<void> {
        // Windows 粘贴模拟：使用 PowerShell 发送 Ctrl+V
        return new Promise((resolve) => {
            const ps = spawn('powershell', [
                '-NoProfile',
                '-Command',
                `Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.SendKeys]::SendWait('^v')`
            ]);

            ps.on('close', () => {
                resolve();
            });

            ps.on('error', (err: Error) => {
                console.error('Paste simulation failed:', err);
                resolve(); // 即使失败也不阻塞
            });
        });
    }
}
