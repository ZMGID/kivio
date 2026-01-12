export interface PlatformAdapter {
    platformName: 'darwin' | 'win32' | 'linux';

    /**
     * Capture a screenshot of a selected area.
     * @param mode 'selection' (default) or 'full'
     * @returns Path to the temporary image file, or null if cancelled.
     */
    captureScreenshot(mode?: 'selection' | 'full'): Promise<string | null>;

    /**
     * Perform local system OCR on the image.
     * @param imagePath Path to the image file.
     * @returns Recognized text.
     */
    performSystemOCR(imagePath: string): Promise<string>;

    /**
     * Get the standard modifier key for the platform.
     * @returns 'Command' for macOS, 'Ctrl' for Windows/Linux.
     */
    getModifierKey(): string;

    /**
     * Register a global hotkey with platform-specific adjustments if needed.
     * @param accelerator The accelerator string (e.g., 'Command+Option+T').
     * @param callback Function to call when hotkey is triggered.
     * @returns true if registration succeeded.
     */
    registerHotkey(accelerator: string, callback: () => void): boolean;

    /**
     * Simulate pasting text into the active application.
     * @param text The text to paste (optional context).
     */
    pasteText(text: string): Promise<void>;
}
