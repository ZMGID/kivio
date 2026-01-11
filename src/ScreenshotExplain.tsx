import { useState, useEffect, useRef, type CSSProperties } from 'react';
import { Send, X, Loader2, ImageIcon, Clock } from 'lucide-react';

interface Message {
    role: 'user' | 'assistant';
    content: string;
}

type AppRegionStyle = CSSProperties & { WebkitAppRegion?: 'drag' | 'no-drag' }
const dragStyle: AppRegionStyle = { WebkitAppRegion: 'drag' }
const noDragStyle: AppRegionStyle = { WebkitAppRegion: 'no-drag' }

function ScreenshotExplain() {
    const [imageId, setImageId] = useState<string>('');
    const [imagePreview, setImagePreview] = useState<string>('');
    const [messages, setMessages] = useState<Message[]>([]);
    const [input, setInput] = useState('');
    const [loading, setLoading] = useState(false);
    const [initializing, setInitializing] = useState(true);
    const [showImage, setShowImage] = useState(true);
    const [showHistory, setShowHistory] = useState(false);
    const [history, setHistory] = useState<Array<{
        id: string;
        timestamp: number;
        messages: Message[];
    }>>([]);
    const messagesEndRef = useRef<HTMLDivElement>(null);

    // Parse image id from URL
    useEffect(() => {
        const hash = window.location.hash;
        const params = new URLSearchParams(hash.split('?')[1] || '');
        const id = params.get('imageId');
        if (id) {
            const decoded = decodeURIComponent(id);
            setImageId(decoded);
            loadImage(decoded);
            getInitialSummary(decoded);
        }
        // Load history
        loadHistory();
    }, []);

    // Load image for preview
    const loadImage = async (id: string) => {
        if (window.api) {
            const result = await window.api.explainReadImage(id);
            if (result.success) {
                setImagePreview(result.data ?? '');
            }
        }
    };

    // Get initial summary
    const getInitialSummary = async (id: string) => {
        setInitializing(true);
        setLoading(true);
        if (window.api) {
            const result = await window.api.explainGetInitialSummary(id);
            if (result.success) {
                setMessages([{ role: 'assistant', content: result.summary ?? '' }]);
            } else {
                setMessages([{ role: 'assistant', content: `错误: ${result.error}` }]);
            }
        }
        setLoading(false);
        setInitializing(false);
    };

    // Auto-scroll to bottom
    useEffect(() => {
        messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
    }, [messages]);

    // Handle send question
    const handleSend = async () => {
        if (!input.trim() || loading) return;

        const userMessage: Message = { role: 'user', content: input };
        setMessages(prev => [...prev, userMessage]);
        setInput('');
        setLoading(true);

        if (window.api) {
            const conversationMessages = [...messages, userMessage];
            const result = await window.api.explainAskQuestion(imageId, conversationMessages);

            if (result.success) {
                setMessages(prev => [...prev, { role: 'assistant', content: result.response ?? '' }]);
            } else {
                setMessages(prev => [...prev, { role: 'assistant', content: `错误: ${result.error}` }]);
            }
        }
        setLoading(false);
    };

    // Handle key down
    const handleKeyDown = (e: React.KeyboardEvent) => {
        if (e.key === 'Enter' && !e.shiftKey) {
            e.preventDefault();
            handleSend();
        } else if (e.key === 'Escape') {
            handleClose();
        }
    };

    // Load history list
    const loadHistory = async () => {
        if (window.api) {
            const result = await window.api.explainGetHistory();
            if (result.success) {
                setHistory(result.history || []);
            }
        }
    };

    // Save current session to history
    const saveToHistory = async () => {
        if (window.api && imageId && messages.length > 0) {
            await window.api.explainSaveHistory(messages);
            await loadHistory();  // Refresh history list
        }
    };

    // Load a history record
    const loadHistoryRecord = async (historyId: string) => {
        if (window.api) {
            const result = await window.api.explainLoadHistory(historyId);
            if (result.success && result.record) {
                setMessages(result.record.messages);
                setShowHistory(false);
                setImagePreview('');  // Clear image since we don't save it
            }
        }
    };

    // Handle close
    const handleClose = async () => {
        // Auto-save before closing if there are messages
        if (messages.length > 0) {
            await saveToHistory();
        }
        if (window.api) {
            window.api.closeExplainWindow();
        }
    };

    return (
        <div className="h-screen w-screen flex flex-col bg-white dark:bg-gray-900">
            {/* Header */}
            <div
                className="flex items-center justify-between px-4 py-3 border-b border-gray-200 dark:border-gray-700 pl-20"
                style={dragStyle}
            >
                <div className="flex items-center space-x-2">
                    <ImageIcon size={20} className="text-gray-600 dark:text-gray-400" />
                    <h1 className="text-lg font-semibold text-gray-900 dark:text-white">截图解释</h1>
                </div>
                <div className="flex items-center space-x-2" style={noDragStyle}>
                    <button
                        onClick={() => setShowHistory(!showHistory)}
                        className="p-1 hover:bg-gray-100 dark:hover:bg-gray-800 rounded transition-colors"
                        title="历史记录"
                    >
                        <Clock size={20} className="text-gray-600 dark:text-gray-400" />
                    </button>
                    <button
                        onClick={handleClose}
                        className="p-1 hover:bg-gray-100 dark:hover:bg-gray-800 rounded transition-colors"
                    >
                        <X size={20} className="text-gray-600 dark:text-gray-400" />
                    </button>
                </div>
            </div>

            {/* History Panel */}
            {showHistory && (
                <div className="border-b border-gray-200 dark:border-gray-700 bg-gray-50 dark:bg-gray-800 max-h-60 overflow-y-auto">
                    <div className="p-4">
                        <h3 className="text-sm font-semibold text-gray-700 dark:text-gray-300 mb-3">历史记录 (最近5条)</h3>
                        {history.length === 0 ? (
                            <p className="text-sm text-gray-500 dark:text-gray-400">暂无历史记录</p>
                        ) : (
                            <div className="space-y-2">
                                {history.map((record) => (
                                    <button
                                        key={record.id}
                                        onClick={() => loadHistoryRecord(record.id)}
                                        className="w-full text-left p-2 rounded hover:bg-gray-100 dark:hover:bg-gray-700 transition-colors"
                                    >
                                        <div className="flex items-center justify-between">
                                            <span className="text-sm text-gray-700 dark:text-gray-300 truncate flex-1">
                                                {record.messages[0]?.content.slice(0, 50)}...
                                            </span>
                                            <span className="text-xs text-gray-500 dark:text-gray-400 ml-2 whitespace-nowrap">
                                                {new Date(record.timestamp).toLocaleDateString('zh-CN', {
                                                    month: 'numeric',
                                                    day: 'numeric',
                                                    hour: '2-digit',
                                                    minute: '2-digit'
                                                })}
                                            </span>
                                        </div>
                                    </button>
                                ))}
                            </div>
                        )}
                    </div>
                </div>
            )}

            {/* Image Preview (Collapsible) */}
            {imagePreview && (
                <div className="border-b border-gray-200 dark:border-gray-700">
                    <button
                        onClick={() => setShowImage(!showImage)}
                        className="w-full px-4 py-2 text-left text-sm text-gray-600 dark:text-gray-400 hover:bg-gray-50 dark:hover:bg-gray-800 transition-colors"
                    >
                        {showImage ? '隐藏图片 ▼' : '显示图片 ▶'}
                    </button>
                    {showImage && (
                        <div className="p-4 bg-gray-50 dark:bg-gray-800">
                            <img
                                src={imagePreview}
                                alt="Screenshot"
                                className="max-w-full max-h-64 mx-auto rounded shadow-lg"
                            />
                        </div>
                    )}
                </div>
            )}

            {/* Messages */}
            <div className="flex-1 overflow-y-auto p-4 space-y-4">
                {initializing && (
                    <div className="flex items-center justify-center py-8">
                        <Loader2 className="animate-spin text-blue-500" size={32} />
                        <span className="ml-2 text-gray-600 dark:text-gray-400">分析图片中...</span>
                    </div>
                )}

                {messages.map((msg, idx) => (
                    <div
                        key={idx}
                        className={`flex ${msg.role === 'user' ? 'justify-end' : 'justify-start'}`}
                    >
                        <div
                            className={`max-w-[80%] px-4 py-2 rounded-lg ${msg.role === 'user'
                                ? 'bg-blue-500 text-white'
                                : 'bg-gray-100 dark:bg-gray-800 text-gray-900 dark:text-white'
                                }`}
                        >
                            <p className="whitespace-pre-wrap">{msg.content}</p>
                        </div>
                    </div>
                ))}

                {loading && !initializing && (
                    <div className="flex justify-start">
                        <div className="bg-gray-100 dark:bg-gray-800 px-4 py-2 rounded-lg">
                            <Loader2 className="animate-spin text-gray-600 dark:text-gray-400" size={20} />
                        </div>
                    </div>
                )}

                <div ref={messagesEndRef} />
            </div>

            {/* Input Area */}
            <div className="border-t border-gray-200 dark:border-gray-700 p-4">
                <div className="flex space-x-2">
                    <textarea
                        value={input}
                        onChange={(e) => setInput(e.target.value)}
                        onKeyDown={handleKeyDown}
                        placeholder="问个问题..."
                        disabled={loading || initializing}
                        className="flex-1 px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-lg resize-none focus:outline-none focus:ring-2 focus:ring-blue-500 bg-white dark:bg-gray-800 text-gray-900 dark:text-white placeholder-gray-400 disabled:opacity-50"
                        rows={2}
                    />
                    <button
                        onClick={handleSend}
                        disabled={!input.trim() || loading || initializing}
                        className="px-4 py-2 bg-blue-500 text-white rounded-lg hover:bg-blue-600 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
                    >
                        <Send size={20} />
                    </button>
                </div>
                <p className="text-xs text-gray-500 dark:text-gray-400 mt-2">
                    按 Enter 发送，Shift+Enter 换行，Esc 关闭
                </p>
            </div>
        </div>
    );
}

export default ScreenshotExplain;
