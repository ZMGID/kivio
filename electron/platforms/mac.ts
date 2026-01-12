import { PlatformAdapter } from './interface';
import { app, globalShortcut } from 'electron';
import { exec } from 'child_process';
import path from 'path';
import fs from 'fs';

export class MacAdapter implements PlatformAdapter {
    platformName = 'darwin' as const;

    async captureScreenshot(mode: 'selection' | 'full' = 'selection'): Promise<string | null> {
        return new Promise((resolve) => {
            const tempPath = path.join(app.getPath('temp'), `screenshot-${Date.now()}.png`);
            const cmd = mode === 'selection' ? `screencapture -i "${tempPath}"` : `screencapture "${tempPath}"`;

            exec(cmd, (error) => {
                if (error || !fs.existsSync(tempPath)) {
                    resolve(null);
                    return;
                }
                resolve(tempPath);
            });
        });
    }

    async performSystemOCR(imagePath: string): Promise<string> {
        return new Promise((resolve, reject) => {
            const isDev = !app.isPackaged;
            const extraResourcePath = isDev
                ? path.join(__dirname, '../../resources/ocr/keylingo-ocr')
                : path.join(process.resourcesPath, 'ocr/keylingo-ocr');

            // Use pre-built Swift binary
            exec(`"${extraResourcePath}" "${imagePath}"`, { maxBuffer: 1024 * 1024 * 10 }, (error, stdout, stderr) => {
                if (error) {
                    console.error('System OCR Error:', stderr);
                    reject(stderr || error.message);
                    return;
                }
                resolve(stdout.trim());
            });
        });
    }

    getModifierKey(): string {
        return 'Command';
    }

    registerHotkey(accelerator: string, callback: () => void): boolean {
        return globalShortcut.register(accelerator, callback);
    }

    async pasteText(text: string): Promise<void> {
        return new Promise((resolve) => {
            // Simulate Command+V
            exec(`osascript -e 'tell application "System Events" to keystroke "v" using command down'`, (error) => {
                if (error) console.error(`Paste failed for text "${text.substring(0, 10)}...":`, error);
                resolve();
            });
        });
    }
}
