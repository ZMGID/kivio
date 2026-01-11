
import { useState, useEffect, type CSSProperties } from 'react'
import { Settings as SettingsIcon, X, Save } from 'lucide-react'

type AppRegionStyle = CSSProperties & { WebkitAppRegion?: 'drag' | 'no-drag' }
const dragStyle: AppRegionStyle = { WebkitAppRegion: 'drag' }
const noDragStyle: AppRegionStyle = { WebkitAppRegion: 'no-drag' }

type SettingsData = {
    hotkey: string;
    theme: 'system' | 'light' | 'dark';
    targetLang: string;
    source: 'bing' | 'openai' | 'custom';
    openai: {
        apiKey: string;
        baseURL: string;
        model: string;
    };
    screenshotTranslation: {
        enabled: boolean;
        hotkey: string;
        ocrSource: 'system' | 'glm';
        glmApiKey: string;
    };
    screenshotExplain: {
        enabled: boolean;
        hotkey: string;
        model: {
            provider: 'glm' | 'openai';
            apiKey: string;
            baseURL: string;
            modelName: string;
        };
        defaultLanguage: 'zh' | 'en';
        customPrompts?: {
            systemPrompt?: string;
            summaryPrompt?: string;
            questionPrompt?: string;
        };
    };
}

interface SettingsProps {
    onClose: () => void;
    onSettingsChange: () => void; // Trigger app to reload settings
}

export default function Settings({ onClose, onSettingsChange }: SettingsProps) {
    const [settings, setSettings] = useState<SettingsData | null>(null);
    const [loading, setLoading] = useState(true);
    const [appVersion, setAppVersion] = useState('');

    useEffect(() => {
        if (window.api) {
            window.api.getSettings().then((data: SettingsData) => {
                setSettings(data);
                setLoading(false);
            });
            window.api.getAppVersion().then((ver: string) => {
                setAppVersion(ver);
            });
        }
    }, []);

    const handleSave = async () => {
        if (!settings || !window.api) return;
        await window.api.saveSettings(settings);
        onSettingsChange();
        onClose();
    };

    if (loading || !settings) return <div className="p-4 text-gray-500">Loading...</div>;

    return (
        <div className="flex flex-col h-full bg-white dark:bg-gray-900 text-gray-800 dark:text-gray-100 p-4 select-none">
            <div className="flex justify-between items-center mb-4 border-b dark:border-gray-700 pb-2"
                style={dragStyle}>
                <h2 className="font-bold text-lg flex items-center gap-2">
                    <SettingsIcon size={18} /> Settings
                </h2>
                <button onClick={onClose} className="p-1 hover:bg-gray-100 dark:hover:bg-gray-800 rounded text-gray-500"
                    style={noDragStyle}>
                    <X size={20} />
                </button>
            </div>

            <div className="flex-1 overflow-auto space-y-4 px-1">

                {/* Visual Settings */}
                <div className="grid grid-cols-2 gap-4">
                    <div>
                        <label className="block text-xs font-bold text-gray-500 uppercase mb-1">Theme</label>
                        <select
                            className="w-full p-2 border dark:border-gray-700 rounded bg-white dark:bg-gray-800 text-sm"
                            value={settings.theme || 'system'}
                            onChange={(e) => setSettings({ ...settings, theme: e.target.value as SettingsData['theme'] })}
                        >
                            <option value="system">Follow System</option>
                            <option value="light">Light</option>
                            <option value="dark">Dark</option>
                        </select>
                    </div>
                    <div>
                        <label className="block text-xs font-bold text-gray-500 uppercase mb-1">Target Language</label>
                        <select
                            className="w-full p-2 border dark:border-gray-700 rounded bg-white dark:bg-gray-800 text-sm"
                            value={settings.targetLang || 'auto'}
                            onChange={(e) => setSettings({ ...settings, targetLang: e.target.value })}
                        >
                            <option value="auto">Auto (ZH ↔ EN)</option>
                            <option value="en">English (En)</option>
                            <option value="zh">Chinese (Zh)</option>
                            <option value="ja">Japanese (Ja)</option>
                            <option value="ko">Korean (Ko)</option>
                            <option value="fr">French (Fr)</option>
                            <option value="de">German (De)</option>
                        </select>
                    </div>
                </div>

                <div className="border-t dark:border-gray-700 my-2"></div>

                {/* Hotkey */}
                <div>
                    <label className="block text-xs font-bold text-gray-500 uppercase mb-1">Shortcut</label>
                    <input
                        className="w-full p-2 border dark:border-gray-700 rounded bg-gray-50 dark:bg-gray-800 text-sm font-mono"
                        value={settings.hotkey}
                        onChange={(e) => setSettings({ ...settings, hotkey: e.target.value })}
                        placeholder="Command+Option+T"
                    />
                </div>

                {/* Source Selection */}
                <div>
                    <label className="block text-xs font-bold text-gray-500 uppercase mb-1">Translation Engine</label>
                    <select
                        className="w-full p-2 border dark:border-gray-700 rounded bg-white dark:bg-gray-800"
                        value={settings.source}
                        onChange={(e) => setSettings({ ...settings, source: e.target.value as SettingsData['source'] })}
                    >
                        <option value="bing">Bing Translate (Free)</option>
                        <option value="openai">DeepSeek / Zhipu / OpenAI (AI)</option>
                    </select>
                </div>

                {/* AI Configuration */}
                {settings.source === 'openai' && (
                    <div className="space-y-3 p-3 bg-gray-50 dark:bg-gray-800 rounded border border-blue-100 dark:border-gray-700">
                        <div>
                            <label className="block text-xs text-gray-600 dark:text-gray-400 mb-1">Base URL</label>
                            <input
                                className="w-full p-2 border dark:border-gray-700 rounded text-sm font-mono dark:bg-gray-900"
                                value={settings.openai.baseURL}
                                onChange={(e) => setSettings({ ...settings, openai: { ...settings.openai, baseURL: e.target.value } })}
                                placeholder="https://api.deepseek.com/v1"
                            />
                        </div>
                        <div>
                            <label className="block text-xs text-gray-600 dark:text-gray-400 mb-1">API Key</label>
                            <input
                                type="password"
                                className="w-full p-2 border dark:border-gray-700 rounded text-sm font-mono dark:bg-gray-900"
                                value={settings.openai.apiKey}
                                onChange={(e) => setSettings({ ...settings, openai: { ...settings.openai, apiKey: e.target.value } })}
                                placeholder="sk-..."
                            />
                        </div>
                        <div>
                            <label className="block text-xs text-gray-600 dark:text-gray-400 mb-1">Model Name</label>
                            <input
                                className="w-full p-2 border dark:border-gray-700 rounded text-sm font-mono dark:bg-gray-900"
                                value={settings.openai.model}
                                onChange={(e) => setSettings({ ...settings, openai: { ...settings.openai, model: e.target.value } })}
                                placeholder="deepseek-chat"
                            />
                        </div>
                    </div>
                )}

                <div className="border-t dark:border-gray-700 my-2"></div>

                {/* Screenshot Translation */}
                <div className="space-y-3 p-3 bg-purple-50/30 dark:bg-purple-900/10 rounded border border-purple-200 dark:border-purple-800">
                    <div className="flex items-center justify-between">
                        <label className="block text-xs font-bold text-gray-700 dark:text-gray-300 uppercase">截图翻译 (Screenshot)</label>
                        <label className="relative inline-flex items-center cursor-pointer">
                            <input
                                type="checkbox"
                                className="sr-only peer"
                                checked={settings.screenshotTranslation?.enabled ?? true}
                                onChange={(e) => setSettings({ ...settings, screenshotTranslation: { ...(settings.screenshotTranslation || { hotkey: 'Command+Shift+A', glmApiKey: '' }), enabled: e.target.checked } })}
                            />
                            <div className="w-9 h-5 bg-gray-200 peer-focus:outline-none peer-focus:ring-2 peer-focus:ring-purple-300 dark:peer-focus:ring-purple-800 rounded-full peer dark:bg-gray-700 peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:border-gray-300 after:border after:rounded-full after:h-4 after:w-4 after:transition-all dark:border-gray-600 peer-checked:bg-purple-600"></div>
                        </label>
                    </div>
                    {settings.screenshotTranslation?.enabled && (
                        <>
                            <div>
                                <label className="block text-xs text-gray-600 dark:text-gray-400 mb-1">Screenshot Hotkey</label>
                                <input
                                    className="w-full p-2 border dark:border-gray-700 rounded bg-white dark:bg-gray-900 text-sm font-mono"
                                    value={settings.screenshotTranslation?.hotkey || 'Command+Shift+A'}
                                    onChange={(e) => setSettings({ ...settings, screenshotTranslation: { ...settings.screenshotTranslation, enabled: true, ocrSource: settings.screenshotTranslation?.ocrSource || 'system', glmApiKey: settings.screenshotTranslation?.glmApiKey || '', hotkey: e.target.value } })}
                                    placeholder="Command+Shift+A"
                                />
                            </div>
                            <div>
                                <label className="block text-xs text-gray-600 dark:text-gray-400 mb-2">OCR 识别源</label>
                                <div className="space-y-2">
                                    <label className="flex items-center gap-2 cursor-pointer">
                                        <input
                                            type="radio"
                                            name="ocrSource"
                                            value="system"
                                            checked={settings.screenshotTranslation?.ocrSource === 'system' || !settings.screenshotTranslation?.ocrSource}
                                            onChange={() => setSettings({ ...settings, screenshotTranslation: { ...settings.screenshotTranslation, enabled: true, hotkey: settings.screenshotTranslation?.hotkey || 'Command+Shift+A', glmApiKey: settings.screenshotTranslation?.glmApiKey || '', ocrSource: 'system' } })}
                                            className="w-4 h-4"
                                        />
                                        <span className="text-sm">系统 OCR (离线，免费)</span>
                                    </label>
                                    <label className="flex items-center gap-2 cursor-pointer">
                                        <input
                                            type="radio"
                                            name="ocrSource"
                                            value="glm"
                                            checked={settings.screenshotTranslation?.ocrSource === 'glm'}
                                            onChange={() => setSettings({ ...settings, screenshotTranslation: { ...settings.screenshotTranslation, enabled: true, hotkey: settings.screenshotTranslation?.hotkey || 'Command+Shift+A', glmApiKey: settings.screenshotTranslation?.glmApiKey || '', ocrSource: 'glm' } })}
                                            className="w-4 h-4"
                                        />
                                        <span className="text-sm">GLM-4V (在线，精度更高)</span>
                                    </label>
                                </div>
                            </div>
                            {settings.screenshotTranslation?.ocrSource === 'glm' && (
                                <div>
                                    <label className="block text-xs text-gray-600 dark:text-gray-400 mb-1">GLM API Key (智谱AI)</label>
                                    <input
                                        type="password"
                                        className="w-full p-2 border dark:border-gray-700 rounded bg-white dark:bg-gray-900 text-sm font-mono"
                                        value={settings.screenshotTranslation?.glmApiKey || ''}
                                        onChange={(e) => setSettings({ ...settings, screenshotTranslation: { ...settings.screenshotTranslation, enabled: true, hotkey: settings.screenshotTranslation?.hotkey || 'Command+Shift+A', ocrSource: 'glm', glmApiKey: e.target.value } })}
                                        placeholder="从 bigmodel.cn 获取"
                                    />
                                    <p className="text-xs text-gray-400 mt-1">访问 <a href="#" onClick={() => window.api?.openExternal('https://bigmodel.cn/console/apikey')} className="text-blue-500 hover:underline">bigmodel.cn</a> 获取免费 API Key</p>
                                </div>
                            )}
                        </>
                    )}
                </div>

                {/* Screenshot Explanation Settings */}
                <div className="mb-4">
                    <div className="flex items-center justify-between mb-2">
                        <label className="text-sm font-medium">截图解释</label>
                        <label className="relative inline-flex items-center cursor-pointer">
                            <input
                                type="checkbox"
                                checked={settings.screenshotExplain?.enabled !== false}
                                onChange={(e) => setSettings({ ...settings, screenshotExplain: { ...settings.screenshotExplain, enabled: e.target.checked, hotkey: settings.screenshotExplain?.hotkey || 'Command+Shift+E', model: settings.screenshotExplain?.model || { provider: 'glm', apiKey: '', baseURL: 'https://open.bigmodel.cn/api/paas/v4', modelName: 'glm-4v-flash' }, defaultLanguage: settings.screenshotExplain?.defaultLanguage || 'zh' } })}
                                className="sr-only peer"
                            />
                            <div className="w-11 h-6 bg-gray-200 rounded-full peer peer-checked:bg-blue-600 peer-focus:ring-2 peer-focus:ring-blue-300 dark:bg-gray-700 dark:peer-focus:ring-blue-800 after:content-[''] after:absolute after:top-0.5 after:left-[2px] after:bg-white after:rounded-full after:h-5 after:w-5 after:transition-all peer-checked:after:translate-x-full"></div>
                        </label>
                    </div>
                    <div className="pl-2 space-y-4">
                        {settings.screenshotExplain?.enabled !== false && (
                            <>
                                <div>
                                    <label className="block text-xs text-gray-600 dark:text-gray-400 mb-2">快捷键</label>
                                    <input
                                        type="text"
                                        value={settings.screenshotExplain?.hotkey || 'Command+Shift+E'}
                                        onChange={(e) => setSettings({ ...settings, screenshotExplain: { ...settings.screenshotExplain, enabled: true, hotkey: e.target.value, model: settings.screenshotExplain?.model || { provider: 'glm', apiKey: '', baseURL: 'https://open.bigmodel.cn/api/paas/v4', modelName: 'glm-4v-flash' }, defaultLanguage: settings.screenshotExplain?.defaultLanguage || 'zh' } })}
                                        placeholder="Command+Shift+E"
                                        className="w-full px-3 py-2 text-sm border dark:border-gray-600 rounded focus:border-blue-500 dark:bg-gray-700"
                                    />
                                </div>
                                <div>
                                    <label className="block text-xs text-gray-600 dark:text-gray-400 mb-2">回复语言</label>
                                    <div className="space-y-2">
                                        <label className="flex items-center gap-2 cursor-pointer">
                                            <input
                                                type="radio"
                                                name="explainLanguage"
                                                value="zh"
                                                checked={settings.screenshotExplain?.defaultLanguage === 'zh' || !settings.screenshotExplain?.defaultLanguage}
                                                onChange={() => setSettings({ ...settings, screenshotExplain: { ...settings.screenshotExplain, enabled: true, hotkey: settings.screenshotExplain?.hotkey || 'Command+Shift+E', model: settings.screenshotExplain?.model || { provider: 'glm', apiKey: '', baseURL: 'https://open.bigmodel.cn/api/paas/v4', modelName: 'glm-4v-flash' }, defaultLanguage: 'zh' } })}
                                                className="w-4 h-4"
                                            />
                                            <span className="text-sm">中文</span>
                                        </label>
                                        <label className="flex items-center gap-2 cursor-pointer">
                                            <input
                                                type="radio"
                                                name="explainLanguage"
                                                value="en"
                                                checked={settings.screenshotExplain?.defaultLanguage === 'en'}
                                                onChange={() => setSettings({ ...settings, screenshotExplain: { ...settings.screenshotExplain, enabled: true, hotkey: settings.screenshotExplain?.hotkey || 'Command+Shift+E', model: settings.screenshotExplain?.model || { provider: 'glm', apiKey: '', baseURL: 'https://open.bigmodel.cn/api/paas/v4', modelName: 'glm-4v-flash' }, defaultLanguage: 'en' } })}
                                                className="w-4 h-4"
                                            />
                                            <span className="text-sm">English</span>
                                        </label>
                                    </div>
                                </div>
                                <div>
                                    <label className="block text-xs text-gray-600 dark:text-gray-400 mb-2">视觉模型</label>
                                    <div className="space-y-2">
                                        <label className="flex items-center gap-2 cursor-pointer">
                                            <input
                                                type="radio"
                                                name="explainProvider"
                                                value="glm"
                                                checked={settings.screenshotExplain?.model?.provider === 'glm' || !settings.screenshotExplain?.model?.provider}
                                                onChange={() => setSettings({ ...settings, screenshotExplain: { ...settings.screenshotExplain, enabled: true, hotkey: settings.screenshotExplain?.hotkey || 'Command+Shift+E', defaultLanguage: settings.screenshotExplain?.defaultLanguage || 'zh', model: { provider: 'glm', apiKey: settings.screenshotExplain?.model?.apiKey || '', baseURL: 'https://open.bigmodel.cn/api/paas/v4', modelName: 'glm-4v-flash' } } })}
                                                className="w-4 h-4"
                                            />
                                            <span className="text-sm">GLM-4V (推荐)</span>
                                        </label>
                                        <label className="flex items-center gap-2 cursor-pointer">
                                            <input
                                                type="radio"
                                                name="explainProvider"
                                                value="openai"
                                                checked={settings.screenshotExplain?.model?.provider === 'openai'}
                                                onChange={() => setSettings({ ...settings, screenshotExplain: { ...settings.screenshotExplain, enabled: true, hotkey: settings.screenshotExplain?.hotkey || 'Command+Shift+E', defaultLanguage: settings.screenshotExplain?.defaultLanguage || 'zh', model: { provider: 'openai', apiKey: settings.screenshotExplain?.model?.apiKey || '', baseURL: settings.screenshotExplain?.model?.baseURL || 'https://api.openai.com/v1', modelName: settings.screenshotExplain?.model?.modelName || 'gpt-4-vision-preview' } } })}
                                                className="w-4 h-4"
                                            />
                                            <span className="text-sm">OpenAI / 自定义</span>
                                        </label>
                                    </div>
                                </div>
                                {settings.screenshotExplain?.model?.provider === 'openai' && (
                                    <div className="space-y-3 pl-6 border-l-2 border-gray-200 dark:border-gray-700">
                                        <div>
                                            <label className="block text-xs text-gray-600 dark:text-gray-400 mb-1">Base URL</label>
                                            <input
                                                type="text"
                                                value={settings.screenshotExplain?.model?.baseURL || 'https://api.openai.com/v1'}
                                                onChange={(e) => setSettings({ ...settings, screenshotExplain: { ...settings.screenshotExplain, enabled: true, hotkey: settings.screenshotExplain?.hotkey || 'Command+Shift+E', defaultLanguage: settings.screenshotExplain?.defaultLanguage || 'zh', model: { ...settings.screenshotExplain.model, provider: 'openai', baseURL: e.target.value } } })}
                                                placeholder="https://api.openai.com/v1"
                                                className="w-full px-2 py-1 text-xs border dark:border-gray-600 rounded focus:border-blue-500 dark:bg-gray-700"
                                            />
                                        </div>
                                        <div>
                                            <label className="block text-xs text-gray-600 dark:text-gray-400 mb-1">Model Name</label>
                                            <input
                                                type="text"
                                                value={settings.screenshotExplain?.model?.modelName || 'gpt-4-vision-preview'}
                                                onChange={(e) => setSettings({ ...settings, screenshotExplain: { ...settings.screenshotExplain, enabled: true, hotkey: settings.screenshotExplain?.hotkey || 'Command+Shift+E', defaultLanguage: settings.screenshotExplain?.defaultLanguage || 'zh', model: { ...settings.screenshotExplain.model, provider: 'openai', modelName: e.target.value } } })}
                                                placeholder="gpt-4-vision-preview"
                                                className="w-full px-2 py-1 text-xs border dark:border-gray-600 rounded focus:border-blue-500 dark:bg-gray-700"
                                            />
                                        </div>
                                        <div>
                                            <label className="block text-xs text-gray-600 dark:text-gray-400 mb-1">API Key</label>
                                            <input
                                                type="password"
                                                value={settings.screenshotExplain?.model?.apiKey || ''}
                                                onChange={(e) => setSettings({ ...settings, screenshotExplain: { ...settings.screenshotExplain, enabled: true, hotkey: settings.screenshotExplain?.hotkey || 'Command+Shift+E', defaultLanguage: settings.screenshotExplain?.defaultLanguage || 'zh', model: { ...settings.screenshotExplain.model, provider: 'openai', apiKey: e.target.value } } })}
                                                placeholder="sk-..."
                                                className="w-full px-2 py-1 text-xs border dark:border-gray-600 rounded focus:border-blue-500 dark:bg-gray-700"
                                            />
                                        </div>
                                    </div>
                                )}
                                {(settings.screenshotExplain?.model?.provider === 'glm' || !settings.screenshotExplain?.model?.provider) && (
                                    <div>
                                        <label className="block text-xs text-gray-600 dark:text-gray-400 mb-1">GLM API Key</label>
                                        <input
                                            type="password"
                                            value={settings.screenshotExplain?.model?.apiKey || ''}
                                            onChange={(e) => setSettings({ ...settings, screenshotExplain: { ...settings.screenshotExplain, enabled: true, hotkey: settings.screenshotExplain?.hotkey || 'Command+Shift+E', defaultLanguage: settings.screenshotExplain?.defaultLanguage || 'zh', model: { ...settings.screenshotExplain.model, provider: 'glm', apiKey: e.target.value, baseURL: 'https://open.bigmodel.cn/api/paas/v4', modelName: 'glm-4v-flash' } } })}
                                            placeholder="输入 API Key"
                                            className="w-full px-3 py-2 text-sm border dark:border-gray-600 rounded focus:border-blue-500 dark:bg-gray-700"
                                        />
                                        <p className="text-xs text-gray-400 mt-1">访问 <a href="#" onClick={() => window.api?.openExternal('https://bigmodel.cn/console/apikey')} className="text-blue-500 hover:underline">bigmodel.cn</a> 获取免费 API Key</p>
                                    </div>
                                )}
                            </>
                        )}
                    </div>

                    {/* Custom Prompts Section */}
                    {settings.screenshotExplain?.enabled !== false && (
                        <div className="mt-4 pt-4 border-t border-gray-200 dark:border-gray-700">
                            <details className="group">
                                <summary className="cursor-pointer text-sm font-medium text-gray-700 dark:text-gray-300 hover:text-blue-500 list-none flex items-center justify-between">
                                    <span>🎨 自定义提示词 (可选)</span>
                                    <span className="text-xs text-gray-500 group-open:hidden">▼ 点击展开</span>
                                </summary>
                                <div className="mt-3 space-y-3 pl-2">
                                    <div>
                                        <label className="block text-xs text-gray-600 dark:text-gray-400 mb-1">系统提示词</label>
                                        <textarea
                                            value={settings.screenshotExplain?.customPrompts?.systemPrompt || ''}
                                            onChange={(e) => setSettings({ ...settings, screenshotExplain: { ...settings.screenshotExplain, enabled: true, hotkey: settings.screenshotExplain?.hotkey || 'Command+Shift+E', defaultLanguage: settings.screenshotExplain?.defaultLanguage || 'zh', model: settings.screenshotExplain?.model || { provider: 'glm', apiKey: '', baseURL: 'https://open.bigmodel.cn/api/paas/v4', modelName: 'glm-4v-flash' }, customPrompts: { ...settings.screenshotExplain?.customPrompts, systemPrompt: e.target.value } } })}
                                            placeholder="留空使用默认值。例如：你是一个专业的图片分析助手..."
                                            className="w-full px-2 py-1.5 text-xs border dark:border-gray-600 rounded focus:border-blue-500 dark:bg-gray-700 font-mono"
                                            rows={2}
                                        />
                                    </div>
                                    <div>
                                        <label className="block text-xs text-gray-600 dark:text-gray-400 mb-1">初始总结提示词</label>
                                        <textarea
                                            value={settings.screenshotExplain?.customPrompts?.summaryPrompt || ''}
                                            onChange={(e) => setSettings({ ...settings, screenshotExplain: { ...settings.screenshotExplain, enabled: true, hotkey: settings.screenshotExplain?.hotkey || 'Command+Shift+E', defaultLanguage: settings.screenshotExplain?.defaultLanguage || 'zh', model: settings.screenshotExplain?.model || { provider: 'glm', apiKey: '', baseURL: 'https://open.bigmodel.cn/api/paas/v4', modelName: 'glm-4v-flash' }, customPrompts: { ...settings.screenshotExplain?.customPrompts, summaryPrompt: e.target.value } } })}
                                            placeholder="留空使用默认值。首次分析图片时使用的提示词..."
                                            className="w-full px-2 py-1.5 text-xs border dark:border-gray-600 rounded focus:border-blue-500 dark:bg-gray-700 font-mono"
                                            rows={3}
                                        />
                                    </div>
                                    <p className="text-xs text-gray-500 dark:text-gray-400 italic">💡 提示：留空将使用默认提示词。自定义提示词可以控制AI的回复风格和格式。</p>
                                </div>
                            </details>
                        </div>
                    )}
                </div>
            </div>

            <div className="pt-4 mt-2 border-t dark:border-gray-700 flex justify-between items-center">
                <div className="text-xs text-gray-400">
                    Ver: {appVersion}
                </div>
                <div className="flex gap-2">
                    <button onClick={onClose} className="px-3 py-1 text-sm text-gray-500 hover:text-gray-700 dark:hover:text-gray-300">Cancel</button>
                    <button onClick={handleSave} className="px-3 py-1 bg-blue-600 text-white rounded text-sm font-medium hover:bg-blue-700 flex items-center gap-1">
                        <Save size={14} /> Save
                    </button>
                </div>
            </div>
        </div>
    )
}
