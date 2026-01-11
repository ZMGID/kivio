import { useState, useEffect, type CSSProperties } from 'react'
import { X, Copy, CheckCircle } from 'lucide-react'
import './index.css'

type AppRegionStyle = CSSProperties & { WebkitAppRegion?: 'drag' | 'no-drag' }
const dragStyle: AppRegionStyle = { WebkitAppRegion: 'drag' }
const noDragStyle: AppRegionStyle = { WebkitAppRegion: 'no-drag' }

export default function ScreenshotResult() {
    const [status, setStatus] = useState<'processing' | 'ready' | 'error'>('processing')
    const [original, setOriginal] = useState('')
    const [translated, setTranslated] = useState('')
    const [error, setError] = useState('')
    const [copied, setCopied] = useState(false)

    useEffect(() => {
        if (!window.api) return;

        // Listen for processing status
        const cleanup1 = window.api.onScreenshotProcessing(() => {
            setStatus('processing');
        });

        // Listen for results
        const cleanup2 = window.api.onScreenshotResult((data) => {
            setOriginal(data.original);
            setTranslated(data.translated);
            setStatus('ready');
        });

        // Listen for errors
        const cleanup3 = window.api.onScreenshotError((errorMsg) => {
            setError(errorMsg);
            setStatus('error');
        });

        // ESC key to close
        const handleKeyDown = (e: KeyboardEvent) => {
            if (e.key === 'Escape') {
                handleClose();
            }
        };
        window.addEventListener('keydown', handleKeyDown);

        return () => {
            cleanup1?.();
            cleanup2?.();
            cleanup3?.();
            window.removeEventListener('keydown', handleKeyDown);
        };
    }, []);

    const handleClose = () => {
        if (window.api) {
            window.api.closeScreenshotWindow();
        }
    };

    const handleCopy = () => {
        if (!navigator.clipboard) return;

        navigator.clipboard.writeText(translated).then(() => {
            setCopied(true);
            setTimeout(() => setCopied(false), 2000);
        });
    };

    return (
        <div className="h-screen w-screen flex flex-col bg-white dark:bg-gray-900 text-gray-900 dark:text-gray-100 select-none"
            style={dragStyle}>

            {/* Header */}
            <div className="flex justify-between items-center px-4 py-3 border-b dark:border-gray-700">
                <h2 className="font-bold text-lg">截图翻译</h2>
                <button
                    onClick={handleClose}
                    className="p-1 text-gray-400 hover:text-gray-600 dark:hover:text-gray-300 rounded"
                    style={noDragStyle}
                >
                    <X size={20} />
                </button>
            </div>

            {/* Content */}
            <div className="flex-1 overflow-auto p-4" style={noDragStyle}>

                {status === 'processing' && (
                    <div className="flex flex-col items-center justify-center h-full">
                        <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-blue-600"></div>
                        <p className="mt-4 text-gray-500">正在识别并翻译...</p>
                    </div>
                )}

                {status === 'error' && (
                    <div className="flex flex-col items-center justify-center h-full">
                        <p className="text-red-500">错误：{error}</p>
                        <button
                            onClick={handleClose}
                            className="mt-4 px-4 py-2 bg-gray-200 dark:bg-gray-700 rounded hover:bg-gray-300 dark:hover:bg-gray-600"
                        >
                            关闭
                        </button>
                    </div>
                )}

                {status === 'ready' && (
                    <div className="space-y-4">
                        {/* Original Text */}
                        {original && (
                            <div>
                                <h3 className="text-sm font-bold text-gray-500 uppercase mb-2">识别原文</h3>
                                <div className="p-3 bg-gray-50 dark:bg-gray-800 rounded border dark:border-gray-700 text-sm whitespace-pre-wrap select-text">
                                    {original}
                                </div>
                            </div>
                        )}

                        {/* Translated Text */}
                        <div>
                            <div className="flex justify-between items-center mb-2">
                                <h3 className="text-sm font-bold text-gray-500 uppercase">翻译结果</h3>
                                <button
                                    onClick={handleCopy}
                                    className="flex items-center gap-1 px-2 py-1 text-xs bg-blue-600 text-white rounded hover:bg-blue-700"
                                >
                                    {copied ? (
                                        <>
                                            <CheckCircle size={14} />
                                            已复制
                                        </>
                                    ) : (
                                        <>
                                            <Copy size={14} />
                                            复制
                                        </>
                                    )}
                                </button>
                            </div>
                            <div className="p-3 bg-blue-50 dark:bg-blue-900/20 rounded border border-blue-200 dark:border-blue-800 text-sm whitespace-pre-wrap select-text">
                                {translated}
                            </div>
                        </div>
                    </div>
                )}
            </div>
        </div>
    )
}
